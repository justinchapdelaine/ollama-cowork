use ollama_cowork_core::{ArtifactPublisher, BrokerError};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use zip::ZipArchive;

pub struct ExclusiveDocxPublisher {
    pub max_bytes: u64,
}

impl ExclusiveDocxPublisher {
    pub fn new(max_bytes: u64) -> Self {
        Self { max_bytes }
    }
}

pub fn validate_docx_artifact(path: &Path, max_bytes: u64) -> Result<(), BrokerError> {
    let metadata = fs::metadata(path).map_err(|e| BrokerError::Publication(e.to_string()))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > max_bytes {
        return Err(BrokerError::Publication("artifact size is invalid".into()));
    }
    let file = fs::File::open(path).map_err(|e| BrokerError::Publication(e.to_string()))?;
    let mut zip = ZipArchive::new(file)
        .map_err(|e| BrokerError::Publication(format!("invalid DOCX ZIP: {e}")))?;
    for name in ["[Content_Types].xml", "_rels/.rels", "word/document.xml"] {
        zip.by_name(name)
            .map_err(|_| BrokerError::Publication(format!("missing DOCX part {name}")))?;
    }
    Ok(())
}

impl ArtifactPublisher for ExclusiveDocxPublisher {
    fn publish(
        &self,
        private_artifact: &Path,
        publish_directory: &Path,
    ) -> Result<PathBuf, BrokerError> {
        validate_docx_artifact(private_artifact, self.max_bytes)?;
        let directory = fs::canonicalize(publish_directory)
            .map_err(|e| BrokerError::Publication(e.to_string()))?;
        let source_name = private_artifact
            .file_stem()
            .and_then(|v| v.to_str())
            .ok_or_else(|| BrokerError::Publication("artifact has no valid name".into()))?;
        for sequence in 1..=9999 {
            let name = if sequence == 1 {
                format!("{source_name}.docx")
            } else {
                format!("{source_name}-{sequence}.docx")
            };
            let target = directory.join(name);
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
            {
                Ok(mut out) => {
                    let mut input = fs::File::open(private_artifact)
                        .map_err(|e| BrokerError::Publication(e.to_string()))?;
                    let copied = std::io::copy(&mut input, &mut out)
                        .map_err(|e| BrokerError::Publication(e.to_string()))?;
                    out.flush()
                        .map_err(|e| BrokerError::Publication(e.to_string()))?;
                    if copied == 0 {
                        let _ = fs::remove_file(&target);
                        return Err(BrokerError::Publication("empty artifact copy".into()));
                    }
                    return Ok(target);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(BrokerError::Publication(e.to_string())),
            }
        }
        Err(BrokerError::Publication(
            "no exclusive artifact name available".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::{ZipWriter, write::SimpleFileOptions};
    fn docx(path: &Path) {
        let f = fs::File::create(path).unwrap();
        let mut z = ZipWriter::new(f);
        for n in ["[Content_Types].xml", "_rels/.rels", "word/document.xml"] {
            z.start_file(n, SimpleFileOptions::default()).unwrap();
            z.write_all(b"<xml/>").unwrap();
        }
        z.finish().unwrap();
    }
    #[test]
    fn publishes_exclusively_without_overwrite() {
        let t = tempfile::tempdir().unwrap();
        let source = t.path().join("job.revised.docx");
        let out = t.path().join("out");
        fs::create_dir(&out).unwrap();
        docx(&source);
        let p = ExclusiveDocxPublisher::new(1024 * 1024);
        let one = p.publish(&source, &out).unwrap();
        let two = p.publish(&source, &out).unwrap();
        assert_ne!(one, two);
        assert!(one.exists() && two.exists());
    }
    #[test]
    fn rejects_non_docx_package() {
        let t = tempfile::tempdir().unwrap();
        let source = t.path().join("bad.docx");
        fs::write(&source, b"not zip").unwrap();
        assert!(
            ExclusiveDocxPublisher::new(1024)
                .publish(&source, t.path())
                .is_err()
        );
    }
}
