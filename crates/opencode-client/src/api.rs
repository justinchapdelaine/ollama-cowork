use crate::{OpencodeEvent, read_sse};
use reqwest::{
    Url,
    blocking::Client,
    header::{AUTHORIZATION, CONTENT_TYPE},
};
use serde_json::{Value, json};
use std::{io::BufReader, time::Duration};
use thiserror::Error;

#[derive(Clone, Debug)]
pub struct OpencodeApiConfig {
    pub base_url: String,
    pub authorization: String,
    pub timeout: Duration,
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
}

pub struct OpencodeApi {
    config: OpencodeApiConfig,
    client: Client,
}
impl OpencodeApi {
    pub fn new(config: OpencodeApiConfig) -> Result<Self, OpencodeApiError> {
        let url = Url::parse(&config.base_url)
            .map_err(|e| OpencodeApiError::InvalidEndpoint(e.to_string()))?;
        if url.scheme() != "http"
            || url.host_str() != Some("127.0.0.1")
            || url.port().is_none()
            || url.path() != "/"
        {
            return Err(OpencodeApiError::InvalidEndpoint(
                "must be http://127.0.0.1:<port>".into(),
            ));
        }
        let client = Client::builder().connect_timeout(config.timeout).build()?;
        Ok(Self { config, client })
    }
    fn url(&self, path: &str) -> String {
        format!("{}{}", self.config.base_url.trim_end_matches('/'), path)
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
            let body = response.text().unwrap_or_default();
            Err(OpencodeApiError::Status { status: code, body })
        }
    }
    pub fn health(&self) -> Result<Value, OpencodeApiError> {
        Ok(self
            .checked(
                self.client
                    .get(self.url("/global/health"))
                    .header(AUTHORIZATION, &self.config.authorization)
                    .timeout(self.config.timeout)
                    .send()?,
            )?
            .json()?)
    }
    pub fn create_session(&self, title: &str) -> Result<Value, OpencodeApiError> {
        Ok(self
            .checked(
                self.client
                    .post(self.url("/session"))
                    .header(AUTHORIZATION, &self.config.authorization)
                    .json(&json!({"title":title}))
                    .timeout(self.config.timeout)
                    .send()?,
            )?
            .json()?)
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
        Ok(self
            .checked(
                self.client
                    .post(self.url(&format!("/session/{session_id}/abort")))
                    .header(AUTHORIZATION, &self.config.authorization)
                    .timeout(self.config.timeout)
                    .send()?,
            )?
            .json()?)
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
        Ok(self
            .checked(
                self.client
                    .get(self.url(&format!("/session/{session_id}/message")))
                    .header(AUTHORIZATION, &self.config.authorization)
                    .timeout(self.config.timeout)
                    .send()?,
            )?
            .json()?)
    }
    pub fn stream_events(
        &self,
        mut callback: impl FnMut(OpencodeEvent) -> bool,
    ) -> Result<(), OpencodeApiError> {
        let response = self.checked(
            self.client
                .get(self.url("/event"))
                .header(AUTHORIZATION, &self.config.authorization)
                .send()?,
        )?;
        read_sse(BufReader::new(response), &mut callback).map_err(OpencodeApiError::Sse)
    }
}
