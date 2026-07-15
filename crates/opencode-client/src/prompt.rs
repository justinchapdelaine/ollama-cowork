use std::collections::BTreeMap;

const MAX_IDENTIFIER_CHARS: usize = 128;
const MAX_SYSTEM_PROMPT_CHARS: usize = 8 * 1024;

/// Model-facing context and visibility restrictions for one workflow profile.
///
/// This value cannot enable tools. Authorization remains a separate host-owned
/// concern; these overrides only remove named tools from the prompt surface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpencodePromptProfile {
    agent_id: String,
    system_prompt: String,
    tool_overrides: BTreeMap<String, bool>,
}

impl OpencodePromptProfile {
    pub fn restricted(
        agent_id: impl Into<String>,
        system_prompt: impl Into<String>,
        disabled_tools: impl IntoIterator<Item = String>,
    ) -> Result<Self, String> {
        let agent_id = agent_id.into();
        let system_prompt = system_prompt.into();
        validate_identifier(&agent_id, "agent")?;
        let system_chars = system_prompt.chars().count();
        if system_prompt.trim().is_empty()
            || system_chars > MAX_SYSTEM_PROMPT_CHARS
            || system_prompt.contains('\0')
        {
            return Err("opencode workflow system prompt is invalid".into());
        }
        let mut tool_overrides = BTreeMap::new();
        for tool in disabled_tools {
            validate_identifier(&tool, "tool")?;
            if tool_overrides.insert(tool, false).is_some() {
                return Err("opencode workflow contains a duplicate disabled tool".into());
            }
        }
        if tool_overrides.is_empty() {
            return Err("opencode workflow must explicitly disable its out-of-scope tools".into());
        }
        Ok(Self {
            agent_id,
            system_prompt,
            tool_overrides,
        })
    }

    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }

    pub fn tool_overrides(&self) -> &BTreeMap<String, bool> {
        &self.tool_overrides
    }
}

fn validate_identifier(value: &str, kind: &str) -> Result<(), String> {
    if value.is_empty()
        || value.chars().count() > MAX_IDENTIFIER_CHARS
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err(format!("opencode {kind} identifier is invalid"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restricted_profile_can_only_disable_bounded_named_tools() {
        let profile = OpencodePromptProfile::restricted(
            "spike-docx",
            "A document is already attached.",
            ["bash".into()],
        )
        .unwrap();
        assert_eq!(profile.agent_id(), "spike-docx");
        assert!(!profile.tool_overrides()["bash"]);

        assert!(
            OpencodePromptProfile::restricted("bad agent", "context", ["bash".into()]).is_err()
        );
        assert!(
            OpencodePromptProfile::restricted("agent", "context", Vec::<String>::new()).is_err()
        );
        assert!(
            OpencodePromptProfile::restricted("agent", "context", ["unsafe/tool".into()]).is_err()
        );
    }
}
