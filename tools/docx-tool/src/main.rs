mod contract;
mod document;
mod filesystem;
mod package;

use anyhow::{Context, Result};
use contract::{Request, Response};
use std::{
    fs,
    io::{self, Read},
};

fn run() -> Result<Response> {
    let args: Vec<String> = std::env::args().collect();
    let mut input = if args.len() == 3 && args[1] == "--request" {
        fs::read_to_string(&args[2]).context("read JSON request file")?
    } else if args.len() == 1 {
        let mut value = String::new();
        io::stdin()
            .read_to_string(&mut value)
            .context("read JSON request from stdin")?;
        value
    } else {
        anyhow::bail!("usage: ollama-cowork-docx-tool [--request <request.json>]")
    };
    if input.starts_with('\u{feff}') {
        input.remove(0);
    }
    let request: Request = serde_json::from_str(&input).context("parse versioned JSON request")?;
    request.validate()?;
    match request {
        Request::Inspect { input, .. } => document::inspect(&input),
        Request::RewriteSection {
            input,
            output,
            heading,
            replacement_paragraphs,
            ..
        } => document::rewrite_section(&input, &output, &heading, &replacement_paragraphs),
        Request::Validate { input, .. } => document::validate(&input),
    }
}

fn main() {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--version")) {
        println!("ollama-cowork-docx-tool {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let response = run().unwrap_or_else(Response::from_error);
    println!(
        "{}",
        serde_json::to_string(&response).expect("serialize response")
    );
    if !response.is_success() {
        std::process::exit(1);
    }
}
