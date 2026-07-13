use anyhow::{Error, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum Request {
    Inspect {
        schema_version: u32,
        input: PathBuf,
    },
    RewriteSection {
        schema_version: u32,
        input: PathBuf,
        output: PathBuf,
        heading: String,
        replacement_paragraphs: Vec<String>,
    },
    Validate {
        schema_version: u32,
        input: PathBuf,
    },
}

impl Request {
    pub fn validate(&self) -> Result<()> {
        let version = match self {
            Self::Inspect { schema_version, .. }
            | Self::RewriteSection { schema_version, .. }
            | Self::Validate { schema_version, .. } => *schema_version,
        };
        if version != SCHEMA_VERSION {
            bail!("unsupported schema_version {version}")
        }
        if let Self::RewriteSection {
            heading,
            replacement_paragraphs,
            ..
        } = self
        {
            if heading.trim().is_empty() {
                bail!("heading must not be empty")
            }
            if replacement_paragraphs.is_empty() {
                bail!("replacement_paragraphs must not be empty")
            }
            if replacement_paragraphs.iter().any(|p| p.contains('\0')) {
                bail!("replacement paragraph contains NUL")
            }
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct SectionSummary {
    pub heading: String,
    pub paragraphs: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Response {
    Inspected {
        schema_version: u32,
        source_sha256: String,
        sections: Vec<SectionSummary>,
    },
    Rewritten {
        schema_version: u32,
        source_sha256: String,
        output_sha256: String,
        output: PathBuf,
        heading: String,
        replaced_paragraph_count: usize,
    },
    Valid {
        schema_version: u32,
        source_sha256: String,
        sections: Vec<SectionSummary>,
    },
    Error {
        schema_version: u32,
        code: String,
        message: String,
    },
}

impl Response {
    pub fn from_error(error: Error) -> Self {
        Self::Error {
            schema_version: SCHEMA_VERSION,
            code: "docx_tool_error".into(),
            message: format!("{error:#}"),
        }
    }
    pub fn is_success(&self) -> bool {
        !matches!(self, Self::Error { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unknown_schema_version() {
        let request = Request::Inspect {
            schema_version: 99,
            input: "a.docx".into(),
        };
        assert!(request.validate().is_err());
    }
    #[test]
    fn rejects_empty_replacement() {
        let request = Request::RewriteSection {
            schema_version: 1,
            input: "a.docx".into(),
            output: "b.docx".into(),
            heading: "Heading".into(),
            replacement_paragraphs: vec![],
        };
        assert!(request.validate().is_err());
    }
}
