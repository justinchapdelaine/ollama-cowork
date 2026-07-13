mod config;
use config::HostConfig;
use ollama_cowork_broker_transport::{
    ApprovalDecisionKind, ApprovalDecisionRequest, BrokerServerConfig, BrokerService,
    serve_localhost,
};
use ollama_cowork_core::{ApprovalState, BrokerOperation, BrokerResult, DocumentJob, ToolBroker};
use ollama_cowork_runtime::{ExclusiveDocxPublisher, SrtRunner, SrtRunnerConfig};
use std::fs;

struct JobService {
    broker: ToolBroker<SrtRunner, ExclusiveDocxPublisher>,
    job_id: String,
    job_token: String,
    source_sha256: String,
}
impl BrokerService for JobService {
    fn decide(&mut self, decision: ApprovalDecisionRequest) -> Result<(), String> {
        if decision.job_id != self.job_id {
            return Err("approval decision does not match the registered job".into());
        }
        if decision.action_id.trim().is_empty() {
            return Err("approval action id must not be empty".into());
        }
        let state = match decision.decision {
            ApprovalDecisionKind::ApprovedOnce => ApprovalState::ApprovedOnce,
            ApprovalDecisionKind::Rejected => ApprovalState::Rejected,
            ApprovalDecisionKind::Cancelled => ApprovalState::Cancelled,
        };
        self.broker
            .decide(&self.job_id, state)
            .map_err(|e| e.to_string())
    }

    fn execute(&mut self, operation: BrokerOperation) -> Result<BrokerResult, String> {
        self.broker
            .execute(ollama_cowork_core::BrokerRequest {
                schema_version: 1,
                job_id: self.job_id.clone(),
                token: self.job_token.clone(),
                source_sha256: self.source_sha256.clone(),
                operation,
            })
            .map_err(|e| e.to_string())
    }
}

fn main() {
    let path = std::env::args_os()
        .nth(1)
        .expect("usage: broker-host <config.json>");
    let config: HostConfig =
        serde_json::from_slice(&fs::read(path).expect("read config")).expect("parse config");
    assert_eq!(config.schema_version, 1);
    let runner = SrtRunner::new(SrtRunnerConfig {
        node: config.node.clone(),
        bridge: config.srt_bridge.clone(),
        docx_tool: config.docx_tool.clone(),
        srt_win: config.srt_win.clone(),
        read_roots: config.read_roots.clone(),
    });
    let mut broker = ToolBroker::new(runner, ExclusiveDocxPublisher::new(50 * 1024 * 1024));
    broker
        .register(DocumentJob::new(
            config.job_id.clone(),
            &config.job_token,
            config.source,
            config.source_sha256.clone(),
            config.private_output_directory,
            config.publish_directory,
        ))
        .expect("register job");
    println!("broker host listening on http://127.0.0.1:{}", config.port);
    serve_localhost(
        BrokerServerConfig {
            port: config.port,
            execution_auth_token: config.auth_token,
            control_auth_token: config.control_auth_token,
            max_request_bytes: 1024 * 1024,
        },
        JobService {
            broker,
            job_id: config.job_id,
            job_token: config.job_token,
            source_sha256: config.source_sha256,
        },
    )
    .expect("serve broker");
}
