use crate::OpencodeEvent;
use ollama_cowork_core::{ArtifactMetadata, ModelEvent};
use serde_json::Value;
use std::collections::HashMap;

/// Accepts only a validated broker result for the expected job and published DOCX.
pub trait ValidatedArtifactDecoder: Send + Sync {
    fn decode_validated_artifact(&self, output: &str) -> Result<Option<ArtifactMetadata>, String>;
}

pub struct OpencodeEventTranslator {
    session_id: String,
    permission_name: String,
    tool_name: String,
    output_decoder: Box<dyn ValidatedArtifactDecoder>,
    text_parts: HashMap<String, String>,
    tool_states: HashMap<String, String>,
}

impl OpencodeEventTranslator {
    pub fn new(
        session_id: String,
        permission_name: String,
        tool_name: String,
        output_decoder: Box<dyn ValidatedArtifactDecoder>,
    ) -> Self {
        Self {
            session_id,
            permission_name,
            tool_name,
            output_decoder,
            text_parts: HashMap::new(),
            tool_states: HashMap::new(),
        }
    }

    pub fn translate(&mut self, event: OpencodeEvent) -> Vec<ModelEvent> {
        match event.event_type.as_str() {
            "permission.asked" => self.permission(event.properties).into_iter().collect(),
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
        Some(ModelEvent::ApprovalRequested {
            external_id: external_id.into(),
            summary: "Create a revised DOCX copy without changing the original.".into(),
        })
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
        if let Some(delta) = string(properties, "delta")
            && !delta.is_empty()
        {
            self.text_parts
                .entry(id.into())
                .or_default()
                .push_str(delta);
            return Some(ModelEvent::Text(delta.into()));
        }
        let text = string(part, "text")?;
        let previous = self
            .text_parts
            .insert(id.into(), text.into())
            .unwrap_or_default();
        let addition = text.strip_prefix(&previous).unwrap_or(text);
        (!addition.is_empty()).then(|| ModelEvent::Text(addition.into()))
    }

    fn part_delta(&mut self, properties: Value) -> Option<ModelEvent> {
        if string(&properties, "sessionID") != Some(self.session_id.as_str())
            || string(&properties, "field").is_some_and(|field| field != "text")
        {
            return None;
        }
        let id = string(&properties, "partID").or_else(|| string(&properties, "id"))?;
        let delta = string(&properties, "delta")?;
        if delta.is_empty() {
            return None;
        }
        self.text_parts
            .entry(id.into())
            .or_default()
            .push_str(delta);
        Some(ModelEvent::Text(delta.into()))
    }

    fn tool_part(&mut self, part: &Value) -> Vec<ModelEvent> {
        if string(part, "tool") != Some(self.tool_name.as_str()) {
            return vec![ModelEvent::Failed {
                code: "unexpected_tool".into(),
                message: "opencode emitted a tool outside the DOCX allowlist".into(),
            }];
        }
        let Some(external_id) = string(part, "callID").or_else(|| string(part, "id")) else {
            return vec![ModelEvent::Failed {
                code: "invalid_tool_event".into(),
                message: "opencode tool event omitted its call identifier".into(),
            }];
        };
        let state = part.get("state").unwrap_or(&Value::Null);
        let Some(status) = string(state, "status") else {
            return Vec::new();
        };
        if self.tool_states.get(external_id).map(String::as_str) == Some(status) {
            return Vec::new();
        }
        self.tool_states.insert(external_id.into(), status.into());
        match status {
            "running" => vec![ModelEvent::ToolStarted],
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

fn sanitized_failure(_detail: &str, fallback: &str) -> String {
    fallback.into()
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

    fn translator() -> OpencodeEventTranslator {
        OpencodeEventTranslator::new(
            "session".into(),
            "docx_rewrite_section".into(),
            "docx_rewrite_section".into(),
            Box::new(Decoder),
        )
    }

    fn event(event_type: &str, properties: Value) -> OpencodeEvent {
        OpencodeEvent {
            event_type: event_type.into(),
            properties,
        }
    }

    #[test]
    fn translates_only_the_expected_permission() {
        let mut translator = translator();
        let allowed = translator.translate(event(
            "permission.asked",
            json!({"sessionID":"session","id":"permission","permission":"docx_rewrite_section"}),
        ));
        assert!(
            matches!(allowed.as_slice(), [ModelEvent::ApprovalRequested { external_id, .. }] if external_id == "permission")
        );

        let denied = translator.translate(event(
            "permission.asked",
            json!({"sessionID":"session","id":"permission","permission":"bash"}),
        ));
        assert!(
            matches!(denied.as_slice(), [ModelEvent::Failed { code, .. }] if code == "unexpected_permission")
        );
    }

    #[test]
    fn ignores_other_sessions_and_deduplicates_text() {
        let mut translator = translator();
        assert!(
            translator
                .translate(event("session.idle", json!({"sessionID":"other"})))
                .is_empty()
        );
        let first = translator.translate(event(
            "message.part.updated",
            json!({"part":{"id":"part","sessionID":"session","type":"text","text":"Hello"}}),
        ));
        let second = translator.translate(event(
            "message.part.updated",
            json!({"part":{"id":"part","sessionID":"session","type":"text","text":"Hello world"}}),
        ));
        assert!(matches!(first.as_slice(), [ModelEvent::Text(value)] if value == "Hello"));
        assert!(matches!(second.as_slice(), [ModelEvent::Text(value)] if value == " world"));
    }

    #[test]
    fn translates_text_delta_events_without_repeating_the_snapshot() {
        let mut translator = translator();
        let delta = translator.translate(event(
            "message.part.delta",
            json!({"sessionID":"session","partID":"part","field":"text","delta":"Hello"}),
        ));
        let snapshot = translator.translate(event(
            "message.part.updated",
            json!({"part":{"id":"part","sessionID":"session","type":"text","text":"Hello world"}}),
        ));
        assert!(matches!(delta.as_slice(), [ModelEvent::Text(value)] if value == "Hello"));
        assert!(matches!(snapshot.as_slice(), [ModelEvent::Text(value)] if value == " world"));
    }

    #[test]
    fn rejects_unexpected_tools() {
        let mut translator = translator();
        let translated = translator.translate(event("message.part.updated", json!({"part":{"id":"part","callID":"call","sessionID":"session","type":"tool","tool":"bash","state":{"status":"running"}}})));
        assert!(
            matches!(translated.as_slice(), [ModelEvent::Failed { code, .. }] if code == "unexpected_tool")
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
}
