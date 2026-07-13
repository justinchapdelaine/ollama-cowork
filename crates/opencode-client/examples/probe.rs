use ollama_cowork_opencode_client::{OpencodeApi, OpencodeApiConfig};
use serde_json::json;
use std::{env, time::Duration};

fn main() {
    let base = env::var("SPIKE_OPENCODE_URL").expect("SPIKE_OPENCODE_URL");
    let authorization = env::var("SPIKE_OPENCODE_AUTH").expect("SPIKE_OPENCODE_AUTH");
    let api = OpencodeApi::new(OpencodeApiConfig {
        base_url: base,
        authorization,
        timeout: Duration::from_secs(10),
    })
    .unwrap();
    let health = api.health().unwrap();
    let session = api.create_session("Rust client proof").unwrap();
    let id = session.get("id").and_then(|v| v.as_str()).unwrap();
    let messages = api.messages(id).unwrap();
    println!(
        "{}",
        json!({"healthy":health.get("healthy")==Some(&json!(true)),"session_id":id,"message_count":messages.as_array().map(|v|v.len()).unwrap_or(0),"passed":true})
    );
}
