use ollama_cowork_core::{ArtifactMetadata, BROKER_SCHEMA_VERSION};
use ollama_cowork_opencode_client::{
    InspectedSections, ValidatedArtifactDecoder, ValidatedInspectionDecoder,
};
use ollama_cowork_runtime::validate_docx_artifact;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, fs, path::PathBuf};

const DOCX_MEDIA_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
const MAX_INSPECTION_SECTIONS: usize = 128;
const MAX_INSPECTION_TOTAL_CHARS: usize = 256 * 1024;

pub struct TrustedDocxOutputDecoder {
    expected_job_id: String,
    expected_source_sha256: String,
    publish_directory: PathBuf,
    max_bytes: u64,
}

impl TrustedDocxOutputDecoder {
    pub fn new(
        expected_job_id: String,
        expected_source_sha256: String,
        publish_directory: PathBuf,
        max_bytes: u64,
    ) -> Self {
        Self {
            expected_job_id,
            expected_source_sha256,
            publish_directory,
            max_bytes,
        }
    }
}

#[derive(Deserialize)]
struct BrokerEnvelope {
    schema_version: u32,
    job_id: String,
    artifact: Option<PathBuf>,
    result: String,
}

#[derive(Deserialize)]
struct DocxResult {
    schema_version: u32,
    source_sha256: String,
    output_sha256: String,
}

#[derive(Deserialize)]
struct InspectionResult {
    schema_version: u32,
    status: String,
    source_sha256: String,
    sections: Vec<InspectionSection>,
}

#[derive(Deserialize)]
struct InspectionSection {
    heading: String,
    paragraphs: Vec<String>,
}

impl ValidatedArtifactDecoder for TrustedDocxOutputDecoder {
    fn decode_validated_artifact(&self, output: &str) -> Result<Option<ArtifactMetadata>, String> {
        let envelope: BrokerEnvelope =
            serde_json::from_str(output).map_err(|_| "broker output is not valid JSON")?;
        if envelope.schema_version != BROKER_SCHEMA_VERSION
            || envelope.job_id != self.expected_job_id
        {
            return Err("broker output does not match the expected job".into());
        }
        let Some(path) = envelope.artifact else {
            return Ok(None);
        };
        let publish_directory = fs::canonicalize(&self.publish_directory)
            .map_err(|_| "publish directory is unavailable")?;
        let path = fs::canonicalize(path).map_err(|_| "published artifact is unavailable")?;
        if !path.starts_with(&publish_directory)
            || path.extension().and_then(|value| value.to_str()) != Some("docx")
        {
            return Err("published artifact is outside the assigned DOCX directory".into());
        }
        validate_docx_artifact(&path, self.max_bytes).map_err(|error| error.to_string())?;
        let result: DocxResult = serde_json::from_str(&envelope.result)
            .map_err(|_| "DOCX result metadata is invalid")?;
        if result.schema_version != BROKER_SCHEMA_VERSION
            || !result
                .source_sha256
                .eq_ignore_ascii_case(&self.expected_source_sha256)
        {
            return Err("DOCX result schema is unsupported".into());
        }
        let sha256 = format!(
            "{:x}",
            Sha256::digest(
                fs::read(&path).map_err(|_| { "published artifact could not be hashed" })?
            )
        );
        if !sha256.eq_ignore_ascii_case(&result.output_sha256) {
            return Err("published artifact digest does not match the DOCX result".into());
        }
        Ok(Some(ArtifactMetadata {
            path,
            media_type: DOCX_MEDIA_TYPE.into(),
            sha256,
        }))
    }
}

impl ValidatedInspectionDecoder for TrustedDocxOutputDecoder {
    fn decode_validated_inspection(&self, output: &str) -> Result<InspectedSections, String> {
        let envelope: BrokerEnvelope =
            serde_json::from_str(output).map_err(|_| "broker output is not valid JSON")?;
        if envelope.schema_version != BROKER_SCHEMA_VERSION
            || envelope.job_id != self.expected_job_id
            || envelope.artifact.is_some()
        {
            return Err("inspection output does not match the expected job".into());
        }
        let result: InspectionResult = serde_json::from_str(&envelope.result)
            .map_err(|_| "DOCX inspection metadata is invalid")?;
        if result.schema_version != BROKER_SCHEMA_VERSION
            || result.status != "inspected"
            || !result
                .source_sha256
                .eq_ignore_ascii_case(&self.expected_source_sha256)
            || result.sections.len() > MAX_INSPECTION_SECTIONS
        {
            return Err("DOCX inspection does not match the selected source".into());
        }
        let mut sections = HashMap::new();
        let mut total_chars = 0_usize;
        for section in result.sections {
            total_chars = total_chars
                .saturating_add(section.heading.chars().count())
                .saturating_add(
                    section
                        .paragraphs
                        .iter()
                        .map(|paragraph| paragraph.chars().count())
                        .sum::<usize>(),
                );
            if section.heading.trim().is_empty()
                || section.heading.contains('\0')
                || section
                    .paragraphs
                    .iter()
                    .any(|paragraph| paragraph.contains('\0'))
                || total_chars > MAX_INSPECTION_TOTAL_CHARS
                || sections
                    .insert(section.heading, section.paragraphs)
                    .is_some()
            {
                return Err("DOCX inspection sections are invalid".into());
            }
        }
        Ok(sections)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn decoder_fixture() -> (tempfile::TempDir, PathBuf, TrustedDocxOutputDecoder, String) {
        let root = tempfile::tempdir().unwrap();
        let publish = root.path().join("published");
        fs::create_dir(&publish).unwrap();
        let artifact = publish.join("revised.docx");
        fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../tests/fixtures/spike-001-original.docx"),
            &artifact,
        )
        .unwrap();
        let sha256 = format!("{:x}", Sha256::digest(fs::read(&artifact).unwrap()));
        let decoder =
            TrustedDocxOutputDecoder::new("job".into(), sha256.clone(), publish, 50 * 1024 * 1024);
        (root, artifact, decoder, sha256)
    }

    fn output(
        job_id: &str,
        artifact: &Path,
        sha256: &str,
        envelope_schema: u32,
        result_schema: u32,
    ) -> String {
        serde_json::json!({
            "schema_version": envelope_schema,
            "job_id": job_id,
            "artifact": artifact,
            "result": serde_json::json!({
                "schema_version": result_schema,
                "source_sha256": sha256,
                "output_sha256": sha256,
            }).to_string(),
        })
        .to_string()
    }

    #[test]
    fn accepts_only_the_expected_published_docx_and_digest() {
        let (_root, artifact, decoder, sha256) = decoder_fixture();
        let output = output(
            "job",
            &artifact,
            &sha256,
            BROKER_SCHEMA_VERSION,
            BROKER_SCHEMA_VERSION,
        );
        let decoded = decoder.decode_validated_artifact(&output).unwrap().unwrap();
        assert_eq!(decoded.sha256, sha256);
        assert_eq!(decoded.media_type, DOCX_MEDIA_TYPE);
    }

    #[test]
    fn rejects_wrong_job_and_digest() {
        let (_root, artifact, decoder, _sha256) = decoder_fixture();
        for (job_id, sha256) in [("other", "abc"), ("job", "abc")] {
            let output = output(
                job_id,
                &artifact,
                sha256,
                BROKER_SCHEMA_VERSION,
                BROKER_SCHEMA_VERSION,
            );
            assert!(decoder.decode_validated_artifact(&output).is_err());
        }
    }

    #[test]
    fn rejects_artifacts_outside_the_assigned_directory_and_sibling_prefixes() {
        let (root, source, decoder, sha256) = decoder_fixture();
        for directory in [
            root.path().join("outside"),
            root.path().join("published-sibling"),
        ] {
            fs::create_dir(&directory).unwrap();
            let artifact = directory.join("revised.docx");
            fs::copy(&source, &artifact).unwrap();
            let output = output(
                "job",
                &artifact,
                &sha256,
                BROKER_SCHEMA_VERSION,
                BROKER_SCHEMA_VERSION,
            );
            assert!(decoder.decode_validated_artifact(&output).is_err());
        }
    }

    #[test]
    fn rejects_non_docx_content_inside_the_assigned_directory() {
        let (root, _source, decoder, _sha256) = decoder_fixture();
        let artifact = root.path().join("published/broken.docx");
        fs::write(&artifact, b"not a DOCX package").unwrap();
        let sha256 = format!("{:x}", Sha256::digest(fs::read(&artifact).unwrap()));
        let output = output(
            "job",
            &artifact,
            &sha256,
            BROKER_SCHEMA_VERSION,
            BROKER_SCHEMA_VERSION,
        );
        assert!(decoder.decode_validated_artifact(&output).is_err());
    }

    #[test]
    fn rejects_unsupported_envelope_and_result_schema_versions() {
        let (_root, artifact, decoder, sha256) = decoder_fixture();
        for (envelope_schema, result_schema) in [
            (BROKER_SCHEMA_VERSION + 1, BROKER_SCHEMA_VERSION),
            (BROKER_SCHEMA_VERSION, BROKER_SCHEMA_VERSION + 1),
        ] {
            let output = output("job", &artifact, &sha256, envelope_schema, result_schema);
            assert!(decoder.decode_validated_artifact(&output).is_err());
        }
    }

    fn inspection_output(job_id: &str, source_sha256: &str) -> String {
        serde_json::json!({
            "schema_version": BROKER_SCHEMA_VERSION,
            "job_id": job_id,
            "artifact": null,
            "result": serde_json::json!({
                "schema_version": BROKER_SCHEMA_VERSION,
                "status": "inspected",
                "source_sha256": source_sha256,
                "sections": [{"heading":"Summary","paragraphs":["Current summary."]}],
            }).to_string(),
        })
        .to_string()
    }

    #[test]
    fn inspection_is_correlated_to_the_expected_job_and_source() {
        let (_root, _artifact, decoder, source_sha256) = decoder_fixture();
        let sections = decoder
            .decode_validated_inspection(&inspection_output("job", &source_sha256))
            .unwrap();
        assert_eq!(sections["Summary"], ["Current summary."]);
        assert!(
            decoder
                .decode_validated_inspection(&inspection_output("other", &source_sha256))
                .is_err()
        );
        assert!(
            decoder
                .decode_validated_inspection(&inspection_output("job", "wrong-source"))
                .is_err()
        );
    }
}
