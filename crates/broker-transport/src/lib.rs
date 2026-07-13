use ollama_cowork_core::{BROKER_SCHEMA_VERSION, BrokerOperation, BrokerRequest, BrokerResult};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    net::{Ipv4Addr, SocketAddrV4},
};
use subtle::ConstantTimeEq;
use tiny_http::{Header, Method, Response, Server, StatusCode};

#[derive(Deserialize)]
pub struct ExecuteRequest {
    pub schema_version: u32,
    #[serde(flatten)]
    pub operation: BrokerOperation,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecisionKind {
    ApprovedOnce,
    Rejected,
    Cancelled,
}

#[derive(Debug, Deserialize)]
pub struct ApprovalDecisionRequest {
    pub schema_version: u32,
    pub job_id: String,
    pub action_id: String,
    pub decision: ApprovalDecisionKind,
}

pub fn authorized(header: Option<&str>, token: &str) -> bool {
    let expected = format!("Bearer {token}");
    let left: [u8; 32] = Sha256::digest(header.unwrap_or("").as_bytes()).into();
    let right: [u8; 32] = Sha256::digest(expected.as_bytes()).into();
    bool::from(left.ct_eq(&right))
}

pub fn broker_request(
    body: ExecuteRequest,
    job_id: &str,
    job_token: &str,
    source_sha256: &str,
) -> Result<BrokerRequest, &'static str> {
    if body.schema_version != BROKER_SCHEMA_VERSION {
        return Err("unsupported schema version");
    }
    Ok(BrokerRequest {
        schema_version: BROKER_SCHEMA_VERSION,
        job_id: job_id.into(),
        token: job_token.into(),
        source_sha256: source_sha256.into(),
        operation: body.operation,
    })
}

pub trait BrokerService: Send {
    fn decide(&mut self, decision: ApprovalDecisionRequest) -> Result<(), String>;
    fn execute(&mut self, operation: BrokerOperation) -> Result<BrokerResult, String>;
}

pub struct BrokerServerConfig {
    pub port: u16,
    pub execution_auth_token: String,
    pub control_auth_token: String,
    pub max_request_bytes: u64,
}

fn validate_server_config(config: &BrokerServerConfig) -> Result<(), String> {
    if config.execution_auth_token.is_empty() || config.control_auth_token.is_empty() {
        return Err("broker auth tokens must not be empty".into());
    }
    if config.execution_auth_token == config.control_auth_token {
        return Err("execution and control auth tokens must differ".into());
    }
    Ok(())
}

fn response(status: u16, value: serde_json::Value) -> Response<std::io::Cursor<Vec<u8>>> {
    let mut response =
        Response::from_data(serde_json::to_vec(&value).expect("serialize broker response"))
            .with_status_code(StatusCode(status));
    response
        .add_header(Header::from_bytes("Content-Type", "application/json").expect("static header"));
    response
}

pub fn serve_localhost(
    config: BrokerServerConfig,
    mut service: impl BrokerService,
) -> Result<(), String> {
    validate_server_config(&config)?;
    let server = Server::http(SocketAddrV4::new(Ipv4Addr::LOCALHOST, config.port))
        .map_err(|e| e.to_string())?;
    for mut request in server.incoming_requests() {
        let auth = request
            .headers()
            .iter()
            .find(|h| h.field.equiv("Authorization"))
            .map(|h| h.value.as_str());
        let decision_route = request.method() == &Method::Post && request.url() == "/decision";
        let expected_token = if decision_route {
            &config.control_auth_token
        } else {
            &config.execution_auth_token
        };
        if !authorized(auth, expected_token) {
            let _ = request.respond(response(401, json!({"error":"unauthorized"})));
            continue;
        }
        if request.method() == &Method::Get && request.url() == "/health" {
            let _ = request.respond(response(200, json!({"schema_version":1,"healthy":true})));
            continue;
        }
        if decision_route {
            let mut bytes = Vec::new();
            if request
                .as_reader()
                .take(config.max_request_bytes + 1)
                .read_to_end(&mut bytes)
                .is_err()
                || bytes.len() as u64 > config.max_request_bytes
            {
                let _ = request.respond(response(400, json!({"error":"invalid_body"})));
                continue;
            }
            let decision: ApprovalDecisionRequest = match serde_json::from_slice(&bytes) {
                Ok(value) => value,
                Err(error) => {
                    let _ = request.respond(response(
                        400,
                        json!({"error":"invalid_request","message":error.to_string()}),
                    ));
                    continue;
                }
            };
            if decision.schema_version != BROKER_SCHEMA_VERSION {
                let _ = request.respond(response(400, json!({"error":"unsupported_schema"})));
                continue;
            }
            match service.decide(decision) {
                Ok(()) => {
                    let _ = request.respond(response(200, json!({"accepted":true})));
                }
                Err(error) => {
                    let _ = request.respond(response(409, json!({"error":error})));
                }
            }
            continue;
        }
        if request.method() != &Method::Post || request.url() != "/execute" {
            let _ = request.respond(response(404, json!({"error":"not_found"})));
            continue;
        }
        let mut bytes = Vec::new();
        if request
            .as_reader()
            .take(config.max_request_bytes + 1)
            .read_to_end(&mut bytes)
            .is_err()
            || bytes.len() as u64 > config.max_request_bytes
        {
            let _ = request.respond(response(400, json!({"error":"invalid_body"})));
            continue;
        }
        let body: ExecuteRequest = match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(error) => {
                let _ = request.respond(response(
                    400,
                    json!({"error":"invalid_request","message":error.to_string()}),
                ));
                continue;
            }
        };
        if body.schema_version != BROKER_SCHEMA_VERSION {
            let _ = request.respond(response(400, json!({"error":"unsupported_schema"})));
            continue;
        }
        match service.execute(body.operation) {
            Ok(value) => {
                let _ = request.respond(response(
                    200,
                    serde_json::to_value(value).expect("serialize result"),
                ));
            }
            Err(error) => {
                let _ = request.respond(response(409, json!({"error":error})));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bearer_auth_is_exact() {
        assert!(authorized(Some("Bearer secret"), "secret"));
        assert!(!authorized(Some("Bearer secret2"), "secret"));
        assert!(!authorized(None, "secret"));
    }

    #[test]
    fn execution_and_control_credentials_must_differ() {
        let config = BrokerServerConfig {
            port: 0,
            execution_auth_token: "same".into(),
            control_auth_token: "same".into(),
            max_request_bytes: 1024,
        };
        assert!(validate_server_config(&config).is_err());
    }
}
