use crate::{OpencodeEvent, SseDecoder};
use futures_util::StreamExt;
use reqwest::{
    Url,
    blocking::Client,
    header::{AUTHORIZATION, CONTENT_TYPE},
    redirect::Policy,
};
use serde_json::{Value, json};
use std::{fmt, io::Read, time::Duration};
use thiserror::Error;

const MAX_ERROR_BODY_BYTES: usize = 8 * 1024;
const MAX_HEALTH_BODY_BYTES: usize = 64 * 1024;
const MAX_CONFIG_BODY_BYTES: usize = 1024 * 1024;
const MAX_SESSION_BODY_BYTES: usize = 256 * 1024;
const MAX_MESSAGES_BODY_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone)]
pub struct OpencodeApiConfig {
    pub base_url: String,
    pub authorization: String,
    pub timeout: Duration,
}

impl fmt::Debug for OpencodeApiConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpencodeApiConfig")
            .field("base_url", &self.base_url)
            .field("authorization", &"[REDACTED]")
            .field("timeout", &self.timeout)
            .finish()
    }
}

#[derive(Debug, Error)]
pub enum OpencodeApiError {
    #[error("invalid opencode endpoint: {0}")]
    InvalidEndpoint(String),
    #[error("opencode HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("opencode API returned {status}: {body}")]
    Status { status: u16, body: String },
    #[error("opencode SSE error: {0}")]
    Sse(String),
    #[error("opencode {route} response exceeded {limit} bytes")]
    BodyTooLarge { route: &'static str, limit: usize },
    #[error("opencode {route} returned invalid JSON: {message}")]
    InvalidJson {
        route: &'static str,
        message: String,
    },
    #[error("opencode response body read failed: {0}")]
    BodyRead(String),
}

pub struct OpencodeApi {
    config: OpencodeApiConfig,
    client: Client,
    stream_client: reqwest::Client,
}

#[derive(Clone)]
pub struct StreamCancellation {
    state: tokio::sync::watch::Sender<bool>,
}

impl Default for StreamCancellation {
    fn default() -> Self {
        let (state, _) = tokio::sync::watch::channel(false);
        Self { state }
    }
}

impl StreamCancellation {
    pub fn cancel(&self) {
        self.state.send_replace(true);
    }
    pub fn is_cancelled(&self) -> bool {
        *self.state.borrow()
    }
    async fn cancelled(&self) {
        let mut state = self.state.subscribe();
        if *state.borrow() {
            return;
        }
        let _ = state.wait_for(|cancelled| *cancelled).await;
    }
}
impl OpencodeApi {
    pub fn base_url(&self) -> &str {
        &self.config.base_url
    }

    pub fn new(config: OpencodeApiConfig) -> Result<Self, OpencodeApiError> {
        let url = Url::parse(&config.base_url)
            .map_err(|e| OpencodeApiError::InvalidEndpoint(e.to_string()))?;
        if url.scheme() != "http"
            || url.host_str() != Some("127.0.0.1")
            || url.port().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(OpencodeApiError::InvalidEndpoint(
                "must be http://127.0.0.1:<port>".into(),
            ));
        }
        if config.authorization.trim().is_empty() || config.timeout.is_zero() {
            return Err(OpencodeApiError::InvalidEndpoint(
                "authorization and timeout must be present".into(),
            ));
        }
        let client = Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .connect_timeout(config.timeout)
            .build()?;
        let stream_client = reqwest::Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .connect_timeout(config.timeout)
            .build()?;
        Ok(Self {
            config,
            client,
            stream_client,
        })
    }
    fn url(&self, path: &str) -> String {
        format!("{}{}", self.config.base_url.trim_end_matches('/'), path)
    }

    fn bounded_error_body(response: reqwest::blocking::Response) -> String {
        let mut bytes = Vec::new();
        let _ = response
            .take(MAX_ERROR_BODY_BYTES as u64)
            .read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes).into_owned()
    }

    async fn bounded_async_error_body(
        response: reqwest::Response,
        cancellation: &StreamCancellation,
    ) -> Result<Option<String>, OpencodeApiError> {
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while bytes.len() < MAX_ERROR_BODY_BYTES {
            let chunk = tokio::select! {
                _ = cancellation.cancelled() => return Ok(None),
                chunk = stream.next() => chunk,
            };
            let Some(chunk) = chunk else { break };
            let chunk = chunk?;
            let remaining = MAX_ERROR_BODY_BYTES - bytes.len();
            bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
            if chunk.len() >= remaining {
                break;
            }
        }
        Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
    }

    fn checked(
        &self,
        response: reqwest::blocking::Response,
    ) -> Result<reqwest::blocking::Response, OpencodeApiError> {
        let status = response.status();
        if status.is_success() {
            Ok(response)
        } else {
            let code = status.as_u16();
            let body = Self::bounded_error_body(response);
            Err(OpencodeApiError::Status { status: code, body })
        }
    }

    fn bounded_json(
        response: reqwest::blocking::Response,
        route: &'static str,
        limit: usize,
    ) -> Result<Value, OpencodeApiError> {
        let mut bytes = Vec::new();
        response
            .take((limit + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| OpencodeApiError::BodyRead(error.to_string()))?;
        if bytes.len() > limit {
            return Err(OpencodeApiError::BodyTooLarge { route, limit });
        }
        serde_json::from_slice(&bytes).map_err(|error| OpencodeApiError::InvalidJson {
            route,
            message: error.to_string(),
        })
    }
    pub fn health(&self) -> Result<Value, OpencodeApiError> {
        Self::bounded_json(
            self.checked(
                self.client
                    .get(self.url("/global/health"))
                    .header(AUTHORIZATION, &self.config.authorization)
                    .timeout(self.config.timeout)
                    .send()?,
            )?,
            "/global/health",
            MAX_HEALTH_BODY_BYTES,
        )
    }
    pub fn config(&self) -> Result<Value, OpencodeApiError> {
        Self::bounded_json(
            self.checked(
                self.client
                    .get(self.url("/config"))
                    .header(AUTHORIZATION, &self.config.authorization)
                    .timeout(self.config.timeout)
                    .send()?,
            )?,
            "/config",
            MAX_CONFIG_BODY_BYTES,
        )
    }
    pub fn create_session(&self, title: &str) -> Result<Value, OpencodeApiError> {
        Self::bounded_json(
            self.checked(
                self.client
                    .post(self.url("/session"))
                    .header(AUTHORIZATION, &self.config.authorization)
                    .json(&json!({"title":title}))
                    .timeout(self.config.timeout)
                    .send()?,
            )?,
            "/session",
            MAX_SESSION_BODY_BYTES,
        )
    }
    pub fn prompt_async(
        &self,
        session_id: &str,
        provider: &str,
        model: &str,
        text: &str,
    ) -> Result<(), OpencodeApiError> {
        self.checked(self.client.post(self.url(&format!("/session/{session_id}/prompt_async"))).header(AUTHORIZATION,&self.config.authorization).header(CONTENT_TYPE,"application/json").json(&json!({"model":{"providerID":provider,"modelID":model},"parts":[{"type":"text","text":text}]})).timeout(self.config.timeout).send()?)?;
        Ok(())
    }
    pub fn abort(&self, session_id: &str) -> Result<bool, OpencodeApiError> {
        let value = Self::bounded_json(
            self.checked(
                self.client
                    .post(self.url(&format!("/session/{session_id}/abort")))
                    .header(AUTHORIZATION, &self.config.authorization)
                    .timeout(self.config.timeout)
                    .send()?,
            )?,
            "/session/:id/abort",
            MAX_HEALTH_BODY_BYTES,
        )?;
        serde_json::from_value(value).map_err(|error| OpencodeApiError::InvalidJson {
            route: "/session/:id/abort",
            message: error.to_string(),
        })
    }
    pub fn reply_permission(&self, request_id: &str, reply: &str) -> Result<(), OpencodeApiError> {
        if !matches!(reply, "once" | "reject") {
            return Err(OpencodeApiError::InvalidEndpoint(
                "invalid permission reply".into(),
            ));
        }
        self.checked(
            self.client
                .post(self.url(&format!("/permission/{request_id}/reply")))
                .header(AUTHORIZATION, &self.config.authorization)
                .json(&json!({"reply":reply}))
                .timeout(self.config.timeout)
                .send()?,
        )?;
        Ok(())
    }
    pub fn messages(&self, session_id: &str) -> Result<Value, OpencodeApiError> {
        Self::bounded_json(
            self.checked(
                self.client
                    .get(self.url(&format!("/session/{session_id}/message")))
                    .header(AUTHORIZATION, &self.config.authorization)
                    .timeout(self.config.timeout)
                    .send()?,
            )?,
            "/session/:id/message",
            MAX_MESSAGES_BODY_BYTES,
        )
    }
    pub async fn stream_events(
        &self,
        cancellation: &StreamCancellation,
        on_ready: impl FnOnce(),
        mut callback: impl FnMut(OpencodeEvent) -> bool,
    ) -> Result<(), OpencodeApiError> {
        let request = self
            .stream_client
            .get(self.url("/event"))
            .header(AUTHORIZATION, &self.config.authorization)
            .send();
        let response = tokio::select! {
            _ = cancellation.cancelled() => return Ok(()),
            response = request => response?,
        };
        let status = response.status();
        if !status.is_success() {
            let Some(body) = Self::bounded_async_error_body(response, cancellation).await? else {
                return Ok(());
            };
            return Err(OpencodeApiError::Status {
                status: status.as_u16(),
                body,
            });
        }
        on_ready();
        let mut decoder = SseDecoder::default();
        let mut stream = response.bytes_stream();
        loop {
            tokio::select! {
                _ = cancellation.cancelled() => return Ok(()),
                chunk = stream.next() => match chunk {
                    Some(Ok(chunk)) => {
                        for event in decoder.push(&chunk).map_err(OpencodeApiError::Sse)? {
                            if !callback(event) { return Ok(()); }
                        }
                    }
                    Some(Err(error)) => return Err(OpencodeApiError::Http(error)),
                    None => return Ok(()),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
    };

    #[test]
    fn config_debug_redacts_authorization() {
        let config = OpencodeApiConfig {
            base_url: "http://127.0.0.1:43123".into(),
            authorization: "Bearer synthetic-opencode-token".into(),
            timeout: Duration::from_secs(1),
        };
        let debug = format!("{config:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("synthetic-opencode-token"));
    }

    fn config(base_url: &str) -> OpencodeApiConfig {
        OpencodeApiConfig {
            base_url: base_url.into(),
            authorization: "Bearer synthetic-opencode-token".into(),
            timeout: Duration::from_secs(1),
        }
    }

    #[test]
    fn rejects_non_root_urls_and_incomplete_configuration() {
        for base_url in [
            "http://127.0.0.1:43123/?query=true",
            "http://127.0.0.1:43123/#fragment",
            "http://user:password@127.0.0.1:43123/",
        ] {
            assert!(OpencodeApi::new(config(base_url)).is_err());
        }

        let mut missing_authorization = config("http://127.0.0.1:43123");
        missing_authorization.authorization.clear();
        assert!(OpencodeApi::new(missing_authorization).is_err());

        let mut zero_timeout = config("http://127.0.0.1:43123");
        zero_timeout.timeout = Duration::ZERO;
        assert!(OpencodeApi::new(zero_timeout).is_err());
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

        let api = OpencodeApi::new(config(&origin)).unwrap();
        let error = api.health().unwrap_err();
        server.join().unwrap();
        assert!(matches!(
            error,
            OpencodeApiError::Status { status: 307, .. }
        ));
    }

    #[test]
    fn rejects_oversized_successful_config_responses() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).unwrap();
            let body = vec![b' '; MAX_CONFIG_BODY_BYTES + 1];
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
        });
        let api = OpencodeApi::new(config(&origin)).unwrap();
        assert!(matches!(
            api.config(),
            Err(OpencodeApiError::BodyTooLarge {
                route: "/config",
                ..
            })
        ));
        server.join().unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_cancellation_wakes_all_waiters_within_a_bound() {
        let cancellation = StreamCancellation::default();
        let first = cancellation.clone();
        let second = cancellation.clone();
        let first_task = tokio::spawn(async move { first.cancelled().await });
        let second_task = tokio::spawn(async move { second.cancelled().await });
        cancellation.cancel();
        for task in [first_task, second_task] {
            tokio::time::timeout(Duration::from_secs(1), task)
                .await
                .expect("cancellation waiter timed out")
                .expect("cancellation waiter failed");
        }

        tokio::time::timeout(Duration::from_secs(1), cancellation.cancelled())
            .await
            .expect("late cancellation waiter timed out");
    }

    #[test]
    fn cancellation_interrupts_a_stalled_sse_error_body() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let (headers_sent, headers_received) = mpsc::sync_channel(1);
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 100\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
            headers_sent.send(()).unwrap();
            let _ = stream.read(&mut request);
        });

        let cancellation = StreamCancellation::default();
        let stream_cancellation = cancellation.clone();
        let (result_sent, result_received) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let api = OpencodeApi::new(config(&origin)).unwrap();
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let result = runtime.block_on(api.stream_events(&stream_cancellation, || {}, |_| true));
            drop(runtime);
            result_sent.send(result).unwrap();
        });
        headers_received
            .recv_timeout(Duration::from_secs(1))
            .expect("error response headers were not sent");
        cancellation.cancel();
        let result = result_received
            .recv_timeout(Duration::from_secs(1))
            .expect("stalled error response ignored cancellation");
        assert!(result.is_ok());
        worker.join().unwrap();
        server.join().unwrap();
    }
}
