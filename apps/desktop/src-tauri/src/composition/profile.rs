use ollama_cowork_opencode_client::OpencodePromptProfile;

pub(super) const SPIKE_001_DOCX_AGENT_ID: &str = "spike-docx";
pub(super) const SPIKE_001_DOCX_SYSTEM_PROMPT: &str = "One DOCX attachment has already been selected and securely bound to this workflow. Use docx_inspect to inspect that attachment; do not search for files or request a path. When the requested section is unambiguous, call docx_rewrite_section with the proposed replacement instead of asking for conversational confirmation. That tool call only requests host approval and cannot create the revised copy until the user chooses Allow once. Use no tools other than docx_inspect and docx_rewrite_section.";

pub(super) fn spike_001_docx_prompt_profile() -> Result<OpencodePromptProfile, String> {
    OpencodePromptProfile::restricted(
        SPIKE_001_DOCX_AGENT_ID,
        SPIKE_001_DOCX_SYSTEM_PROMPT,
        ["bash".into()],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docx_profile_has_no_file_path_and_can_only_hide_tools() {
        let profile = spike_001_docx_prompt_profile().unwrap();
        assert_eq!(profile.agent_id(), SPIKE_001_DOCX_AGENT_ID);
        assert!(!profile.tool_overrides()["bash"]);
        assert!(!profile.system_prompt().contains(":\\"));
        assert!(!profile.system_prompt().contains(".docx"));
    }
}
