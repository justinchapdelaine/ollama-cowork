use serde::Serialize;

pub const DESKTOP_HEALTH_EVENT: &str = "desktop://health";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentHealth {
    pub state: ComponentState,
    pub version: Option<String>,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentState {
    Ready,
    Configured,
    Unavailable,
    Unsupported,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopHealth {
    pub app_version: String,
    pub ready: bool,
    pub opencode: ComponentHealth,
    pub sandbox: ComponentHealth,
    pub model_endpoint: ComponentHealth,
}
