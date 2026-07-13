use ollama_cowork_core::{ApprovalState, BrokerOperation, BrokerRequest, DocumentJob, ToolBroker};
use ollama_cowork_runtime::{ExclusiveDocxPublisher, SrtRunner, SrtRunnerConfig};
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn hash(path: &Path) -> String {
    format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
}
fn request(job: &str, token: &str, source_hash: &str) -> BrokerRequest {
    BrokerRequest {
        schema_version: 1,
        job_id: job.into(),
        token: token.into(),
        source_sha256: source_hash.into(),
        operation: BrokerOperation::RewriteSection {
            heading: "Executive Summary".into(),
            replacement_paragraphs: vec![
                "Rewritten through the trusted Rust broker and SRT adapter.".into(),
                "The source remains immutable and publication is exclusive.".into(),
            ],
        },
    }
}

fn main() {
    let args: Vec<_> = env::args_os().collect();
    if args.len() != 4 {
        eprintln!("usage: broker-proof <repo> <node.exe> <srt-win.exe>");
        std::process::exit(2)
    }
    let repo = PathBuf::from(&args[1]);
    let node = PathBuf::from(&args[2]);
    let srt = PathBuf::from(&args[3]);
    let source = repo.join("tests/fixtures/spike-001-original.docx");
    let source_hash = hash(&source);
    let root = tempfile::tempdir().unwrap();
    let private = root.path().join("private");
    let published = root.path().join("published");
    fs::create_dir(&private).unwrap();
    fs::create_dir(&published).unwrap();
    let runner = SrtRunner::new(SrtRunnerConfig {
        node,
        bridge: repo.join("scripts/runtime/srt-docx-bridge.mjs"),
        docx_tool: repo.join("target/debug/ollama-cowork-docx-tool.exe"),
        srt_win: srt,
        read_roots: vec![repo.clone()],
    });
    let mut broker = ToolBroker::new(runner, ExclusiveDocxPublisher::new(50 * 1024 * 1024));
    let token = "spike-proof-ephemeral-token";
    broker
        .register(DocumentJob::new(
            "allow".into(),
            token,
            source.clone(),
            source_hash.clone(),
            private.clone(),
            published.clone(),
        ))
        .unwrap();
    broker
        .decide(
            "allow",
            "action",
            request("allow", token, &source_hash).operation,
            ApprovalState::ApprovedOnce,
        )
        .unwrap();
    let allowed = broker
        .execute(request("allow", token, &source_hash))
        .unwrap();
    let reject_private = root.path().join("reject-private");
    let reject_publish = root.path().join("reject-publish");
    fs::create_dir(&reject_private).unwrap();
    fs::create_dir(&reject_publish).unwrap();
    broker
        .register(DocumentJob::new(
            "reject".into(),
            token,
            source.clone(),
            source_hash.clone(),
            reject_private.clone(),
            reject_publish.clone(),
        ))
        .unwrap();
    broker
        .decide(
            "reject",
            "action",
            request("reject", token, &source_hash).operation,
            ApprovalState::Rejected,
        )
        .unwrap();
    let rejected = broker
        .execute(request("reject", token, &source_hash))
        .unwrap_err()
        .to_string();
    let cancel_private = root.path().join("cancel-private");
    let cancel_publish = root.path().join("cancel-publish");
    fs::create_dir(&cancel_private).unwrap();
    fs::create_dir(&cancel_publish).unwrap();
    broker
        .register(DocumentJob::new(
            "cancel".into(),
            token,
            source.clone(),
            source_hash.clone(),
            cancel_private.clone(),
            cancel_publish.clone(),
        ))
        .unwrap();
    broker
        .decide(
            "cancel",
            "action",
            request("cancel", token, &source_hash).operation,
            ApprovalState::Cancelled,
        )
        .unwrap();
    let cancelled = broker
        .execute(request("cancel", token, &source_hash))
        .unwrap_err()
        .to_string();
    let source_after = hash(&source);
    let rejected_empty = fs::read_dir(reject_publish).unwrap().next().is_none();
    let cancelled_empty = fs::read_dir(cancel_publish).unwrap().next().is_none();
    let passed = source_hash == source_after
        && allowed.artifact.as_ref().is_some_and(|p| p.exists())
        && rejected_empty
        && cancelled_empty;
    let report = serde_json::json!({"schema_version":1,"source_sha256_before":source_hash,"source_sha256_after":source_after,"allowed":allowed,"rejected":{"error":rejected,"artifact_directory_empty":rejected_empty},"cancelled":{"error":cancelled,"artifact_directory_empty":cancelled_empty},"passed":passed});
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    if !passed {
        std::process::exit(1)
    }
}
