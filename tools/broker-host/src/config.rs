use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
pub struct HostConfig {
    pub schema_version: u32,
    pub port: u16,
    pub auth_token: String,
    pub control_auth_token: String,
    pub job_id: String,
    pub job_token: String,
    pub source: PathBuf,
    pub source_sha256: String,
    pub private_output_directory: PathBuf,
    pub publish_directory: PathBuf,
    pub node: PathBuf,
    pub srt_bridge: PathBuf,
    pub docx_tool: PathBuf,
    pub srt_win: PathBuf,
    pub read_roots: Vec<PathBuf>,
}
