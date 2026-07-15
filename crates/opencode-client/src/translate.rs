use crate::OpencodeEvent;
use ollama_cowork_core::{ActionProposal, ArtifactMetadata, BrokerOperation, ModelEvent};
use serde_json::Value;
use std::collections::HashMap;

const MAX_TRACKED_MESSAGES: usize = 128;
const MAX_TEXT_PARTS: usize = 128;
const MAX_TEXT_PART_CHARS: usize = 32_768;
const MAX_TRACKED_TOOL_CALLS: usize = 128;
const MAX_EVENT_IDENTIFIER_CHARS: usize = 256;

pub type InspectedSections = HashMap<String, Vec<String>>;

/// Accepts only a validated broker result for the expected job and published DOCX.
pub trait ValidatedArtifactDecoder: Send + Sync {
    fn decode_validated_artifact(&self, output: &str) -> Result<Option<ArtifactMetadata>, String>;
}

/// Accepts only a trusted inspection result for the expected job and source DOCX.
pub trait ValidatedInspectionDecoder: Send + Sync {
    fn decode_validated_inspection(&self, output: &str) -> Result<InspectedSections, String>;
}

pub struct OpencodeEventTranslator {
    session_id: String,
    permission_name: String,
    mutation_tool_name: String,
    read_only_tool_name: String,
    output_decoder: Box<dyn ValidatedArtifactDecoder>,
    inspection_decoder: Box<dyn ValidatedInspectionDecoder>,
    message_roles: HashMap<String, String>,
    text_part_messages: HashMap<String, String>,
    text_parts: HashMap<String, String>,
    tool_states: HashMap<String, String>,
    inspected_sections: HashMap<String, Vec<String>>,
}

impl OpencodeEventTranslator {
    pub fn new(
        session_id: String,
        permission_name: String,
        mutation_tool_name: String,
        read_only_tool_name: String,
        output_decoder: Box<dyn ValidatedArtifactDecoder>,
        inspection_decoder: Box<dyn ValidatedInspectionDecoder>,
    ) -> Self {
        Self {
            session_id,
            permission_name,
            mutation_tool_name,
            read_only_tool_name,
            output_decoder,
            inspection_decoder,
            message_roles: HashMap::new(),
            text_part_messages: HashMap::new(),
            text_parts: HashMap::new(),
            tool_states: HashMap::new(),
            inspected_sections: HashMap::new(),
        }
    }

    pub fn translate(&mut self, event: OpencodeEvent) -> Vec<ModelEvent> {
        match event.event_type.as_str() {
            "permission.asked" => self.permission(event.properties).into_iter().collect(),
            "message.updated" => self.message(event.properties).into_iter().collect(),
            "message.part.updated" => self.part(event.properties),
            "message.part.delta" => self.part_delta(event.properties).into_iter().collect(),
            "session.idle" => self
                .matches_session(&event.properties)
                .then_some(ModelEvent::Idle)
                .into_iter()
                .collect(),
            "session.error" => self.failure(event.properties).into_iter().collect(),
            _ => Vec::new(),
        }
    }

    fn permission(&self, properties: Value) -> Option<ModelEvent> {
        if !self.matches_session(&properties) {
            return None;
        }
        let name = string(&properties, "permission").or_else(|| string(&properties, "type"));
        if name != Some(self.permission_name.as_str()) {
            return Some(ModelEvent::Failed {
                code: "unexpected_permission".into(),
                message: "opencode requested a capability outside the DOCX allowlist".into(),
            });
        }
        let external_id = string(&properties, "id").or_else(|| string(&properties, "requestID"));
        let Some(external_id) = external_id else {
            return Some(ModelEvent::Failed {
                code: "invalid_permission_event".into(),
                message: "opencode permission event omitted its request identifier".into(),
            });
        };
        if !valid_event_identifier(external_id) {
            return Some(limit_failure(
                "opencode permission identifier exceeded its configured limit",
            ));
        }
        let metadata = properties.get("metadata");
        let heading = metadata.and_then(|value| string(value, "heading"));
        let paragraphs = metadata
            .and_then(|value| value.get("replacement_paragraphs"))
            .and_then(Value::as_array)
            .and_then(|values| values.iter().map(Value::as_str).collect::<Option<Vec<_>>>());
        if metadata.and_then(|value| string(value, "operation")) != Some("rewrite_section")
            || heading.is_none_or(|value| value.trim().is_empty())
            || paragraphs.as_ref().is_none_or(|values| values.is_empty())
        {
            return Some(ModelEvent::Failed {
                code: "invalid_permission_event".into(),
                message: "opencode permission event omitted the exact DOCX rewrite proposal".into(),
            });
        }
        let heading = heading.unwrap();
        let replacement_paragraphs = paragraphs
            .unwrap()
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let Some(current_paragraphs) = self.inspected_sections.get(heading).cloned() else {
            return Some(ModelEvent::Failed {
                code: "invalid_permission_event".into(),
                message: "The proposed DOCX section was not present in the trusted inspection."
                    .into(),
            });
        };
        Some(ModelEvent::ApprovalRequested {
            external_id: external_id.into(),
            summary: format!("Rewrite the {heading} section in a revised DOCX copy."),
            operation: BrokerOperation::RewriteSection {
                heading: heading.into(),
                replacement_paragraphs: replacement_paragraphs.clone(),
            },
            proposal: ActionProposal::DocxSectionRewrite {
                heading: heading.into(),
                current_paragraphs,
                replacement_paragraphs,
            },
        })
    }

    fn message(&mut self, properties: Value) -> Option<ModelEvent> {
        let info = properties
            .get("info")
            .or_else(|| properties.get("message"))
            .unwrap_or(&properties);
        if string(info, "sessionID") != Some(self.session_id.as_str()) {
            return None;
        }
        let id = string(info, "id").or_else(|| string(info, "messageID"))?;
        if !valid_event_identifier(id) {
            return Some(limit_failure(
                "opencode message identifier exceeded its configured limit",
            ));
        }
        let role = string(info, "role")?;
        if !matches!(role, "assistant" | "user" | "system") {
            return Some(limit_failure(
                "opencode emitted an unsupported message role",
            ));
        }
        if !self.message_roles.contains_key(id) && self.message_roles.len() >= MAX_TRACKED_MESSAGES
        {
            return Some(limit_failure(
                "opencode message tracking exceeded its configured limit",
            ));
        }
        self.message_roles.insert(id.into(), role.into());
        None
    }

    fn part(&mut self, properties: Value) -> Vec<ModelEvent> {
        let Some(part) = properties.get("part") else {
            return Vec::new();
        };
        if string(part, "sessionID") != Some(self.session_id.as_str()) {
            return Vec::new();
        }
        match string(part, "type") {
            Some("text") => self.text_part(&properties, part).into_iter().collect(),
            Some("tool") => self.tool_part(part),
            _ => Vec::new(),
        }
    }

    fn text_part(&mut self, properties: &Value, part: &Value) -> Option<ModelEvent> {
        let id = string(part, "id").or_else(|| string(part, "partID"))?;
        let message_id = string(part, "messageID")?;
        if !valid_event_identifier(id) || !valid_event_identifier(message_id) {
            return Some(limit_failure(
                "opencode text identifier exceeded its configured limit",
            ));
        }
        if self.message_roles.get(message_id).map(String::as_str) != Some("assistant") {
            return None;
        }
        if !self.text_parts.contains_key(id) && self.text_parts.len() >= MAX_TEXT_PARTS {
            return Some(limit_failure(
                "assistant text part tracking exceeded its configured limit",
            ));
        }
        self.text_part_messages.insert(id.into(), message_id.into());
        if let Some(delta) = string(properties, "delta")
            && !delta.is_empty()
        {
            if !append_bounded(self.text_parts.entry(id.into()).or_default(), delta) {
                return Some(limit_failure(
                    "assistant text exceeded its configured limit",
                ));
            }
            return Some(ModelEvent::Text {
                part_id: id.into(),
                text: delta.into(),
            });
        }
        let text = string(part, "text")?;
        if text.chars().count() > MAX_TEXT_PART_CHARS {
            return Some(limit_failure(
                "assistant text exceeded its configured limit",
            ));
        }
        let previous = self
            .text_parts
            .insert(id.into(), text.into())
            .unwrap_or_default();
        let addition = text.strip_prefix(&previous).unwrap_or(text);
        (!addition.is_empty()).then(|| ModelEvent::Text {
            part_id: id.into(),
            text: addition.into(),
        })
    }

    fn part_delta(&mut self, properties: Value) -> Option<ModelEvent> {
        if string(&properties, "sessionID") != Some(self.session_id.as_str())
            || string(&properties, "field").is_some_and(|field| field != "text")
        {
            return None;
        }
        let id = string(&properties, "partID").or_else(|| string(&properties, "id"))?;
        let message_id = string(&properties, "messageID")
            .or_else(|| self.text_part_messages.get(id).map(String::as_str))?;
        if !valid_event_identifier(id) || !valid_event_identifier(message_id) {
            return Some(limit_failure(
                "opencode text identifier exceeded its configured limit",
            ));
        }
        if self.message_roles.get(message_id).map(String::as_str) != Some("assistant") {
            return None;
        }
        let delta = string(&properties, "delta")?;
        if delta.is_empty() {
            return None;
        }
        if !self.text_parts.contains_key(id) && self.text_parts.len() >= MAX_TEXT_PARTS {
            return Some(limit_failure(
                "assistant text part tracking exceeded its configured limit",
            ));
        }
        self.text_part_messages.insert(id.into(), message_id.into());
        if !append_bounded(self.text_parts.entry(id.into()).or_default(), delta) {
            return Some(limit_failure(
                "assistant text exceeded its configured limit",
            ));
        }
        Some(ModelEvent::Text {
            part_id: id.into(),
            text: delta.into(),
        })
    }

    fn tool_part(&mut self, part: &Value) -> Vec<ModelEvent> {
        let tool = string(part, "tool");
        if tool != Some(self.mutation_tool_name.as_str())
            && tool != Some(self.read_only_tool_name.as_str())
        {
            return vec![ModelEvent::Failed {
                code: "unexpected_tool".into(),
                message: format!(
                    "opencode emitted tool '{}' outside the DOCX allowlist",
                    diagnostic_identifier(tool)
                ),
            }];
        }
        let Some(external_id) = string(part, "callID").or_else(|| string(part, "id")) else {
            return vec![ModelEvent::Failed {
                code: "invalid_tool_event".into(),
                message: "opencode tool event omitted its call identifier".into(),
            }];
        };
        if !valid_event_identifier(external_id) {
            return vec![limit_failure(
                "opencode tool identifier exceeded its configured limit",
            )];
        }
        let state = part.get("state").unwrap_or(&Value::Null);
        let Some(status) = string(state, "status") else {
            return Vec::new();
        };
        if !matches!(status, "pending" | "running" | "completed" | "error") {
            return vec![ModelEvent::Failed {
                code: "invalid_tool_event".into(),
                message: "opencode emitted an unsupported tool status".into(),
            }];
        }
        if self.tool_states.get(external_id).map(String::as_str) == Some(status) {
            return Vec::new();
        }
        if !self.tool_states.contains_key(external_id)
            && self.tool_states.len() >= MAX_TRACKED_TOOL_CALLS
        {
            return vec![limit_failure(
                "opencode tool tracking exceeded its configured limit",
            )];
        }
        self.tool_states.insert(external_id.into(), status.into());
        if tool == Some(self.read_only_tool_name.as_str()) {
            return match status {
                "completed" => {
                    match self
                        .inspection_decoder
                        .decode_validated_inspection(string(state, "output").unwrap_or_default())
                    {
                        Ok(sections) => {
                            self.inspected_sections = sections;
                            Vec::new()
                        }
                        Err(_) => vec![ModelEvent::Failed {
                            code: "invalid_inspection_output".into(),
                            message: "The trusted DOCX inspection result could not be validated."
                                .into(),
                        }],
                    }
                }
                "error" => vec![ModelEvent::Failed {
                    code: "inspect_tool_failed".into(),
                    message: "The trusted DOCX inspection failed.".into(),
                }],
                _ => Vec::new(),
            };
        }
        match status {
            // OpenCode marks a custom tool as running before `context.ask`
            // necessarily resolves. Host approval, not this transport state,
            // defines when the trusted mutation begins.
            "pending" | "running" => Vec::new(),
            "completed" => {
                let output = string(state, "output").unwrap_or_default().to_owned();
                let mut events = vec![ModelEvent::ToolCompleted];
                match self.output_decoder.decode_validated_artifact(&output) {
                    Ok(Some(artifact)) => events.push(ModelEvent::ArtifactReady(artifact)),
                    Ok(None) => {}
                    Err(message) => events.push(ModelEvent::Failed {
                        code: "invalid_tool_output".into(),
                        message: sanitized_failure(
                            &message,
                            "The revised document result could not be validated.",
                        ),
                    }),
                }
                events
            }
            "error" => vec![ModelEvent::Failed {
                code: "tool_failed".into(),
                message: "The trusted DOCX operation failed.".into(),
            }],
            _ => Vec::new(),
        }
    }

    fn failure(&self, properties: Value) -> Option<ModelEvent> {
        if !self.matches_session(&properties) {
            return None;
        }
        let message = string(&properties, "message")
            .or_else(|| {
                properties
                    .get("error")
                    .and_then(|error| string(error, "message"))
            })
            .unwrap_or("opencode session failed");
        Some(ModelEvent::Failed {
            code: "opencode_session_failed".into(),
            message: sanitized_failure(message, "The model session failed."),
        })
    }

    fn matches_session(&self, value: &Value) -> bool {
        string(value, "sessionID") == Some(self.session_id.as_str())
    }
}

fn append_bounded(target: &mut String, addition: &str) -> bool {
    if target.chars().count() + addition.chars().count() > MAX_TEXT_PART_CHARS {
        return false;
    }
    target.push_str(addition);
    true
}

fn limit_failure(message: &str) -> ModelEvent {
    ModelEvent::Failed {
        code: "opencode_event_limit_exceeded".into(),
        message: message.into(),
    }
}

fn valid_event_identifier(value: &str) -> bool {
    !value.is_empty()
        && !value.contains('\0')
        && value.chars().count() <= MAX_EVENT_IDENTIFIER_CHARS
}

fn sanitized_failure(_detail: &str, fallback: &str) -> String {
    fallback.into()
}

fn diagnostic_identifier(value: Option<&str>) -> String {
    let Some(value) = value else {
        return "<missing>".into();
    };
    let bounded = value.chars().take(65).collect::<String>();
    if bounded.len() <= 64
        && !bounded.is_empty()
        && bounded
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        bounded
    } else {
        "<invalid>".into()
    }
}

fn string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Decoder;
    impl ValidatedArtifactDecoder for Decoder {
        fn decode_validated_artifact(
            &self,
            output: &str,
        ) -> Result<Option<ArtifactMetadata>, String> {
            Ok((output == "artifact-output").then(|| ArtifactMetadata {
                path: "C:/work/input.revised.docx".into(),
                media_type: "application/docx".into(),
                sha256: "abc".into(),
            }))
        }
    }

    impl ValidatedInspectionDecoder for Decoder {
        fn decode_validated_inspection(&self, output: &str) -> Result<InspectedSections, String> {
            if output != "inspection-output" {
                return Err("invalid inspection fixture".into());
            }
            Ok(HashMap::from([(
                "Summary".into(),
                vec!["Current summary.".into()],
            )]))
        }
    }

    fn translator() -> OpencodeEventTranslator {
        OpencodeEventTranslator::new(
            "session".into(),
            "docx_rewrite_section".into(),
            "docx_rewrite_section".into(),
            "docx_inspect".into(),
            Box::new(Decoder),
            Box::new(Decoder),
        )
    }

    fn event(event_type: &str, properties: Value) -> OpencodeEvent {
        OpencodeEvent {
            event_type: event_type.into(),
            properties,
        }
    }

    fn record_inspection(translator: &mut OpencodeEventTranslator) {
        assert!(translator
            .translate(event(
                "message.part.updated",
                json!({"part":{"id":"inspect-part","callID":"inspect-call","sessionID":"session","type":"tool","tool":"docx_inspect","state":{"status":"completed","output":"inspection-output"}}}),
            ))
            .is_empty());
    }

    fn record_message(translator: &mut OpencodeEventTranslator, id: &str, role: &str) {
        assert!(
            translator
                .translate(event(
                    "message.updated",
                    json!({"info":{"id":id,"sessionID":"session","role":role}}),
                ))
                .is_empty()
        );
    }

    #[test]
    fn translates_only_the_expected_permission() {
        let mut translator = translator();
        record_inspection(&mut translator);
        let allowed = translator.translate(event(
            "permission.asked",
            json!({"sessionID":"session","id":"permission","permission":"docx_rewrite_section","metadata":{"operation":"rewrite_section","heading":"Summary","replacement_paragraphs":["Revised"]}}),
        ));
        assert!(matches!(
            allowed.as_slice(),
            [ModelEvent::ApprovalRequested {
                external_id,
                proposal: ActionProposal::DocxSectionRewrite {
                    heading,
                    current_paragraphs,
                    replacement_paragraphs,
                },
                ..
            }] if external_id == "permission"
                && heading == "Summary"
                && current_paragraphs == &["Current summary."]
                && replacement_paragraphs == &["Revised"]
        ));

        let denied = translator.translate(event(
            "permission.asked",
            json!({"sessionID":"session","id":"permission","permission":"bash"}),
        ));
        assert!(
            matches!(denied.as_slice(), [ModelEvent::Failed { code, .. }] if code == "unexpected_permission")
        );
    }

    #[test]
    fn rejects_permission_without_an_exact_operation_proposal() {
        let mut translator = translator();
        let translated = translator.translate(event(
            "permission.asked",
            json!({"sessionID":"session","id":"permission","permission":"docx_rewrite_section"}),
        ));
        assert!(
            matches!(translated.as_slice(), [ModelEvent::Failed { code, .. }] if code == "invalid_permission_event")
        );
    }

    #[test]
    fn rejects_a_rewrite_proposal_without_a_trusted_inspection_match() {
        let mut translator = translator();
        let translated = translator.translate(event(
            "permission.asked",
            json!({"sessionID":"session","id":"permission","permission":"docx_rewrite_section","metadata":{"operation":"rewrite_section","heading":"Summary","replacement_paragraphs":["Revised"]}}),
        ));
        assert!(matches!(
            translated.as_slice(),
            [ModelEvent::Failed { code, .. }] if code == "invalid_permission_event"
        ));
    }

    #[test]
    fn ignores_other_sessions_and_deduplicates_text() {
        let mut translator = translator();
        record_message(&mut translator, "assistant-message", "assistant");
        assert!(
            translator
                .translate(event("session.idle", json!({"sessionID":"other"})))
                .is_empty()
        );
        let first = translator.translate(event(
            "message.part.updated",
            json!({"part":{"id":"part","messageID":"assistant-message","sessionID":"session","type":"text","text":"Hello"}}),
        ));
        let second = translator.translate(event(
            "message.part.updated",
            json!({"part":{"id":"part","messageID":"assistant-message","sessionID":"session","type":"text","text":"Hello world"}}),
        ));
        assert!(
            matches!(first.as_slice(), [ModelEvent::Text { part_id, text }] if part_id == "part" && text == "Hello")
        );
        assert!(
            matches!(second.as_slice(), [ModelEvent::Text { part_id, text }] if part_id == "part" && text == " world")
        );
    }

    #[test]
    fn translates_text_delta_events_without_repeating_the_snapshot() {
        let mut translator = translator();
        record_message(&mut translator, "assistant-message", "assistant");
        let delta = translator.translate(event(
            "message.part.delta",
            json!({"sessionID":"session","messageID":"assistant-message","partID":"part","field":"text","delta":"Hello"}),
        ));
        let snapshot = translator.translate(event(
            "message.part.updated",
            json!({"part":{"id":"part","messageID":"assistant-message","sessionID":"session","type":"text","text":"Hello world"}}),
        ));
        assert!(matches!(delta.as_slice(), [ModelEvent::Text { text, .. }] if text == "Hello"));
        assert!(matches!(snapshot.as_slice(), [ModelEvent::Text { text, .. }] if text == " world"));
    }

    #[test]
    fn ignores_user_text_and_keeps_assistant_parts_distinct() {
        let mut translator = translator();
        record_message(&mut translator, "user-message", "user");
        record_message(&mut translator, "assistant-message", "assistant");
        assert!(translator
            .translate(event(
                "message.part.updated",
                json!({"part":{"id":"user-part","messageID":"user-message","sessionID":"session","type":"text","text":"Rewrite Summary"}}),
            ))
            .is_empty());
        for (id, text) in [("assistant-one", "First"), ("assistant-two", "Second")] {
            let translated = translator.translate(event(
                "message.part.updated",
                json!({"part":{"id":id,"messageID":"assistant-message","sessionID":"session","type":"text","text":text}}),
            ));
            assert!(matches!(
                translated.as_slice(),
                [ModelEvent::Text { part_id, text: value }] if part_id == id && value == text
            ));
        }
    }

    #[test]
    fn fails_closed_when_one_assistant_part_exceeds_the_memory_bound() {
        let mut translator = translator();
        record_message(&mut translator, "assistant-message", "assistant");
        let translated = translator.translate(event(
            "message.part.updated",
            json!({"part":{"id":"part","messageID":"assistant-message","sessionID":"session","type":"text","text":"x".repeat(MAX_TEXT_PART_CHARS + 1)}}),
        ));
        assert!(matches!(
            translated.as_slice(),
            [ModelEvent::Failed { code, .. }] if code == "opencode_event_limit_exceeded"
        ));
    }

    #[test]
    fn bounds_tool_state_tracking_even_when_calls_never_complete() {
        let mut translator = translator();
        for index in 0..MAX_TRACKED_TOOL_CALLS {
            let translated = translator.translate(event(
                "message.part.updated",
                json!({"part":{"id":format!("part-{index}"),"callID":format!("call-{index}"),"sessionID":"session","type":"tool","tool":"docx_inspect","state":{"status":"pending"}}}),
            ));
            assert!(translated.is_empty(), "{translated:?}");
        }
        let overflow = translator.translate(event(
            "message.part.updated",
            json!({"part":{"id":"overflow","callID":"overflow","sessionID":"session","type":"tool","tool":"docx_inspect","state":{"status":"pending"}}}),
        ));
        assert!(matches!(
            overflow.as_slice(),
            [ModelEvent::Failed { code, .. }] if code == "opencode_event_limit_exceeded"
        ));
    }

    #[test]
    fn rejects_oversized_model_controlled_identifiers_before_tracking_them() {
        let mut translator = translator();
        let oversized = "x".repeat(MAX_EVENT_IDENTIFIER_CHARS + 1);
        for translated in [
            translator.translate(event(
                "message.updated",
                json!({"info":{"id":oversized,"sessionID":"session","role":"assistant"}}),
            )),
            translator.translate(event(
                "message.part.updated",
                json!({"part":{"id":"part","callID":oversized,"sessionID":"session","type":"tool","tool":"docx_inspect","state":{"status":"pending"}}}),
            )),
        ] {
            assert!(matches!(
                translated.as_slice(),
                [ModelEvent::Failed { code, .. }] if code == "opencode_event_limit_exceeded"
            ));
        }
    }

    #[test]
    fn rejects_unexpected_tools() {
        let mut translator = translator();
        let translated = translator.translate(event("message.part.updated", json!({"part":{"id":"part","callID":"call","sessionID":"session","type":"tool","tool":"bash","state":{"status":"running"}}})));
        assert!(
            matches!(translated.as_slice(), [ModelEvent::Failed { code, message }] if code == "unexpected_tool" && message == "opencode emitted tool 'bash' outside the DOCX allowlist")
        );
    }

    #[test]
    fn bounds_unexpected_tool_diagnostics_to_safe_identifiers() {
        assert_eq!(diagnostic_identifier(Some("docx_inspect")), "docx_inspect");
        assert_eq!(diagnostic_identifier(None), "<missing>");
        assert_eq!(
            diagnostic_identifier(Some("unsafe tool\nsecret")),
            "<invalid>"
        );
        assert_eq!(diagnostic_identifier(Some(&"a".repeat(65))), "<invalid>");
    }

    #[test]
    fn accepts_only_the_named_read_only_inspection_tool_without_mutation_events() {
        let mut translator = translator();
        for status in ["pending", "running"] {
            let translated = translator.translate(event(
                "message.part.updated",
                json!({"part":{"id":format!("part-{status}"),"callID":format!("call-{status}"),"sessionID":"session","type":"tool","tool":"docx_inspect","state":{"status":status,"output":"ignored"}}}),
            ));
            assert!(translated.is_empty(), "{translated:?}");
        }
        record_inspection(&mut translator);
        let failed = translator.translate(event(
            "message.part.updated",
            json!({"part":{"id":"part-error","callID":"call-error","sessionID":"session","type":"tool","tool":"docx_inspect","state":{"status":"error"}}}),
        ));
        assert!(
            matches!(failed.as_slice(), [ModelEvent::Failed { code, .. }] if code == "inspect_tool_failed")
        );
    }

    #[test]
    fn translates_completed_artifact_metadata() {
        let mut translator = translator();
        let translated = translator.translate(event("message.part.updated", json!({"part":{"id":"part","callID":"call","sessionID":"session","type":"tool","tool":"docx_rewrite_section","state":{"status":"completed","output":"artifact-output"}}})));
        assert!(
            matches!(translated.as_slice(), [ModelEvent::ToolCompleted, ModelEvent::ArtifactReady(artifact)] if artifact.sha256 == "abc")
        );
    }

    #[test]
    fn mutation_running_is_transport_state_regardless_of_permission_event_order() {
        let mut running_first = translator();
        record_inspection(&mut running_first);
        assert!(
            running_first
                .translate(event(
                    "message.part.updated",
                    json!({"part":{"id":"part","callID":"call","sessionID":"session","type":"tool","tool":"docx_rewrite_section","state":{"status":"running"}}}),
                ))
                .is_empty()
        );
        assert!(matches!(
            running_first.translate(event(
                "permission.asked",
                json!({"sessionID":"session","id":"permission","permission":"docx_rewrite_section","metadata":{"operation":"rewrite_section","heading":"Summary","replacement_paragraphs":["Revised"]}}),
            )).as_slice(),
            [ModelEvent::ApprovalRequested { .. }]
        ));

        let mut permission_first = translator();
        record_inspection(&mut permission_first);
        assert!(matches!(
            permission_first.translate(event(
                "permission.asked",
                json!({"sessionID":"session","id":"permission","permission":"docx_rewrite_section","metadata":{"operation":"rewrite_section","heading":"Summary","replacement_paragraphs":["Revised"]}}),
            )).as_slice(),
            [ModelEvent::ApprovalRequested { .. }]
        ));
        assert!(
            permission_first
                .translate(event(
                    "message.part.updated",
                    json!({"part":{"id":"part","callID":"call","sessionID":"session","type":"tool","tool":"docx_rewrite_section","state":{"status":"running"}}}),
                ))
                .is_empty()
        );
    }
}
