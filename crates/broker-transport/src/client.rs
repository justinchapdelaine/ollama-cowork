use ollama_cowork_core::{
    BROKER_SCHEMA_VERSION, BrokerOperation, MutationAuthorization, MutationDecision,
};
use reqwest::{Url, blocking::Client, header::AUTHORIZATION, redirect::Policy};
use serde_json::{Value, json};
use std::{fmt, io::Read, time::Duration};
use thiserror::Error;

#[derive(Clone)]
pub struct BrokerAuthorizationConfig {
    pub base_url: String,
    pub control_authorization: String,
    pub job_id: String,
    pub timeout: Duration,
}

impl fmt::Debug for BrokerAuthorizationConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BrokerAuthorizationConfig")
            .field("base_url", &self.base_url)
            .field("control_authorization", &"[REDACTED]")
            .field("job_id", &self.job_id)
            .field("timeout", &self.timeout)
            .finish()
    }
}

#[derive(Debug, Error)]
pub enum BrokerAuthorizationError {
    #[error("invalid broker authorization configuration: {0}")]
    InvalidConfig(String),
    #[error("broker authorization HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("broker authorization returned {status}: {body}")]
    Status { status: u16, body: String },
}

pub struct BrokerAuthorizationClient {
    config: BrokerAuthorizationConfig,
    client: Client,
}

impl BrokerAuthorizationClient {
    pub fn new(config: BrokerAuthorizationConfig) -> Result<Self, BrokerAuthorizationError> {
        let url = Url::parse(&config.base_url)
            .map_err(|error| BrokerAuthorizationError::InvalidConfig(error.to_string()))?;
        if url.scheme() != "http"
            || url.host_str() != Some("127.0.0.1")
            || url.port().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(BrokerAuthorizationError::InvalidConfig(
                "base URL must be http://127.0.0.1:<port>".into(),
            ));
        }
        if config.control_authorization.trim().is_empty()
            || config.job_id.trim().is_empty()
            || config.timeout.is_zero()
        {
            return Err(BrokerAuthorizationError::InvalidConfig(
                "authorization, job ID, and timeout must be present".into(),
            ));
        }
        let client = Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .connect_timeout(config.timeout)
            .build()?;
        Ok(Self { config, client })
    }

    fn post(&self, path: &str, body: Value) -> Result<(), BrokerAuthorizationError> {
        let response = self
            .client
            .post(format!(
                "{}{}",
                self.config.base_url.trim_end_matches('/'),
                path
            ))
            .header(AUTHORIZATION, &self.config.control_authorization)
            .json(&body)
            .timeout(self.config.timeout)
            .send()?;
        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status().as_u16();
            let mut body = String::new();
            let _ = response.take(8 * 1024).read_to_string(&mut body);
            Err(BrokerAuthorizationError::Status { status, body })
        }
    }
}

impl MutationAuthorization for BrokerAuthorizationClient {
    fn decide(
        &mut self,
        action_id: &str,
        operation: &BrokerOperation,
        decision: MutationDecision,
    ) -> Result<(), String> {
        let decision = match decision {
            MutationDecision::ApprovedOnce => "approved_once",
            MutationDecision::Rejected => "rejected",
            MutationDecision::Cancelled => "cancelled",
        };
        self.post(
            "/decision",
            json!({
                "schema_version": BROKER_SCHEMA_VERSION,
                "job_id": self.config.job_id,
                "action_id": action_id,
                "decision": decision,
                "operation": operation,
            }),
        )
        .map_err(|error| error.to_string())
    }

    fn revoke_unconsumed(&mut self, action_id: &str) -> Result<(), String> {
        self.post(
            "/revoke",
            json!({
                "schema_version": BROKER_SCHEMA_VERSION,
                "job_id": self.config.job_id,
                "action_id": action_id,
            }),
        )
        .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, net::TcpListener, thread};

    fn config(base_url: &str) -> BrokerAuthorizationConfig {
        BrokerAuthorizationConfig {
            base_url: base_url.into(),
            control_authorization: "Bearer synthetic-control-token".into(),
            job_id: "job".into(),
            timeout: Duration::from_secs(1),
        }
    }

    #[test]
    fn accepts_only_explicit_ipv4_loopback_origins() {
        assert!(BrokerAuthorizationClient::new(config("http://127.0.0.1:43123")).is_ok());
        for invalid in [
            "http://localhost:43123",
            "http://0.0.0.0:43123",
            "https://127.0.0.1:43123",
            "http://127.0.0.1:43123/path",
            "http://user:password@127.0.0.1:43123/",
        ] {
            assert!(BrokerAuthorizationClient::new(config(invalid)).is_err());
        }
    }

    #[test]
    fn rejects_empty_control_material_and_zero_timeout() {
        let mut value = config("http://127.0.0.1:43123");
        value.control_authorization.clear();
        assert!(BrokerAuthorizationClient::new(value).is_err());

        let mut value = config("http://127.0.0.1:43123");
        value.timeout = Duration::ZERO;
        assert!(BrokerAuthorizationClient::new(value).is_err());
    }

    #[test]
    fn redacts_control_authorization_from_debug_output() {
        let value = config("http://127.0.0.1:43123");
        let debug = format!("{value:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("synthetic-control-token"));
    }

    fn capture_requests(count: usize) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let mut requests = Vec::new();
            for _ in 0..count {
                let (mut stream, _) = listener.accept().unwrap();
                let mut bytes = Vec::new();
                let mut chunk = [0_u8; 1024];
                loop {
                    let read = stream.read(&mut chunk).unwrap();
                    bytes.extend_from_slice(&chunk[..read]);
                    let Some(header_end) = bytes.windows(4).position(|part| part == b"\r\n\r\n")
                    else {
                        continue;
                    };
                    let headers = String::from_utf8_lossy(&bytes[..header_end]);
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= header_end + 4 + content_length {
                        break;
                    }
                }
                requests.push(String::from_utf8(bytes).unwrap());
                stream
                    .write_all(
                        b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .unwrap();
            }
            requests
        });
        (origin, server)
    }

    #[test]
    fn sends_versioned_job_correlated_control_contracts() {
        let (origin, server) = capture_requests(2);
        let mut client = BrokerAuthorizationClient::new(config(&origin)).unwrap();
        client
            .decide(
                "action",
                &BrokerOperation::RewriteSection {
                    heading: "Summary".into(),
                    replacement_paragraphs: vec!["Revised".into()],
                },
                MutationDecision::ApprovedOnce,
            )
            .unwrap();
        client.revoke_unconsumed("action").unwrap();

        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("POST /decision HTTP/1.1\r\n"));
        assert!(requests[1].starts_with("POST /revoke HTTP/1.1\r\n"));
        for request in &requests {
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer synthetic-control-token")
            );
        }
        let decision: Value =
            serde_json::from_str(requests[0].split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(
            decision,
            json!({
                "schema_version": BROKER_SCHEMA_VERSION,
                "job_id": "job",
                "action_id": "action",
                "decision": "approved_once",
                "operation": {
                    "operation": "rewrite_section",
                    "heading": "Summary",
                    "replacement_paragraphs": ["Revised"]
                },
            })
        );
        let revocation: Value =
            serde_json::from_str(requests[1].split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(
            revocation,
            json!({
                "schema_version": BROKER_SCHEMA_VERSION,
                "job_id": "job",
                "action_id": "action",
            })
        );
    }

    #[test]
    fn does_not_follow_redirects_away_from_the_configured_origin() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 307 Temporary Redirect\r\nLocation: http://127.0.0.1:9/not-local\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        });

        let client = BrokerAuthorizationClient::new(config(&origin)).unwrap();
        let error = client.post("/decision", json!({})).unwrap_err();
        server.join().unwrap();
        assert!(matches!(
            error,
            BrokerAuthorizationError::Status { status: 307, .. }
        ));
    }
}
