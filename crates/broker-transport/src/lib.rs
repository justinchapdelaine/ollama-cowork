mod client;

use sha2::{Digest, Sha256};

/// Produces an HMAC-SHA256 proof without disclosing the endpoint credential.
pub fn broker_health_proof(token: &str, challenge: &str) -> String {
    let mut key = token.as_bytes().to_vec();
    if key.len() > 64 {
        key = Sha256::digest(&key).to_vec();
    }
    key.resize(64, 0);
    let mut inner_pad = [0x36_u8; 64];
    let mut outer_pad = [0x5c_u8; 64];
    for index in 0..64 {
        inner_pad[index] ^= key[index];
        outer_pad[index] ^= key[index];
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(challenge.as_bytes());
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner.finalize());
    format!("{:x}", outer.finalize())
}

pub use client::{BrokerAuthorizationClient, BrokerAuthorizationConfig, BrokerAuthorizationError};

#[cfg(feature = "server")]
mod server {
    use ollama_cowork_core::{BROKER_SCHEMA_VERSION, BrokerOperation, BrokerRequest, BrokerResult};
    use serde::Deserialize;
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::{
        io::Read,
        net::{Ipv4Addr, SocketAddrV4},
    };
    use subtle::ConstantTimeEq;
    use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

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
        pub operation: BrokerOperation,
    }

    #[derive(Debug, Deserialize)]
    pub struct ApprovalRevocationRequest {
        pub schema_version: u32,
        pub job_id: String,
        pub action_id: String,
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
        fn revoke(&mut self, revocation: ApprovalRevocationRequest) -> Result<(), String>;
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
        response.add_header(
            Header::from_bytes("Content-Type", "application/json").expect("static header"),
        );
        response
    }

    pub fn serve_localhost(
        config: BrokerServerConfig,
        mut service: impl BrokerService,
    ) -> Result<(), String> {
        validate_server_config(&config)?;
        let server = Server::http(SocketAddrV4::new(Ipv4Addr::LOCALHOST, config.port))
            .map_err(|e| e.to_string())?;
        for request in server.incoming_requests() {
            handle_request(&config, &mut service, request);
        }
        Ok(())
    }

    fn handle_request(
        config: &BrokerServerConfig,
        service: &mut impl BrokerService,
        mut request: Request,
    ) {
        if request.method() == &Method::Get
            && let Some(challenge) = request.url().strip_prefix("/health?challenge=")
            && challenge.len() == 64
            && challenge.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            let proof = crate::broker_health_proof(&config.execution_auth_token, challenge);
            let _ = request.respond(response(
                200,
                json!({"schema_version":1,"healthy":true,"proof":proof}),
            ));
            return;
        }
        let auth = request
            .headers()
            .iter()
            .find(|h| h.field.equiv("Authorization"))
            .map(|h| h.value.as_str());
        let control_route =
            request.method() == &Method::Post && matches!(request.url(), "/decision" | "/revoke");
        let expected_token = if control_route {
            &config.control_auth_token
        } else {
            &config.execution_auth_token
        };
        if !authorized(auth, expected_token) {
            let _ = request.respond(response(401, json!({"error":"unauthorized"})));
            return;
        }
        if control_route {
            let mut bytes = Vec::new();
            if request
                .as_reader()
                .take(config.max_request_bytes + 1)
                .read_to_end(&mut bytes)
                .is_err()
                || bytes.len() as u64 > config.max_request_bytes
            {
                let _ = request.respond(response(400, json!({"error":"invalid_body"})));
                return;
            }
            let result = if request.url() == "/decision" {
                serde_json::from_slice::<ApprovalDecisionRequest>(&bytes)
                    .map_err(|error| error.to_string())
                    .and_then(|decision| {
                        if decision.schema_version != BROKER_SCHEMA_VERSION {
                            return Err("unsupported_schema".into());
                        }
                        service.decide(decision)
                    })
            } else {
                serde_json::from_slice::<ApprovalRevocationRequest>(&bytes)
                    .map_err(|error| error.to_string())
                    .and_then(|revocation| {
                        if revocation.schema_version != BROKER_SCHEMA_VERSION {
                            return Err("unsupported_schema".into());
                        }
                        service.revoke(revocation)
                    })
            };
            match result {
                Ok(()) => {
                    let _ = request.respond(response(200, json!({"accepted":true})));
                }
                Err(error) => {
                    let _ = request.respond(response(409, json!({"error":error})));
                }
            }
            return;
        }
        if request.method() != &Method::Post || request.url() != "/execute" {
            let _ = request.respond(response(404, json!({"error":"not_found"})));
            return;
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
            return;
        }
        let body: ExecuteRequest = match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(error) => {
                let _ = request.respond(response(
                    400,
                    json!({"error":"invalid_request","message":error.to_string()}),
                ));
                return;
            }
        };
        if body.schema_version != BROKER_SCHEMA_VERSION {
            let _ = request.respond(response(400, json!({"error":"unsupported_schema"})));
            return;
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
}

#[cfg(feature = "server")]
pub use server::*;
