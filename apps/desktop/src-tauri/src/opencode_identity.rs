use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

pub const VERSION: &str = "1.17.18";
pub const SHA256: &str = "D78D0999EADDF4BAE028FFA88106D37F5962931BB9137396D8C5FD77576DD68D";
pub const FILE_LENGTH: u64 = 179_946_888;

pub fn matches(path: &Path) -> Result<bool, String> {
    matches_expected(path, FILE_LENGTH, SHA256)
}

pub fn matches_expected(
    path: &Path,
    expected_length: u64,
    expected_sha256: &str,
) -> Result<bool, String> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("could not read opencode executable: {error}"))?;
    if file
        .metadata()
        .map_err(|error| format!("could not inspect opencode executable: {error}"))?
        .len()
        != expected_length
    {
        return Ok(false);
    }
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("could not hash opencode executable: {error}"))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:X}", digest.finalize()) == expected_sha256)
}

pub fn select(
    explicit: Option<PathBuf>,
    path_candidate: PathBuf,
    fallbacks: impl IntoIterator<Item = PathBuf>,
) -> PathBuf {
    if let Some(explicit) = explicit {
        return explicit;
    }
    std::iter::once(path_candidate.clone())
        .chain(fallbacks)
        .find(|candidate| {
            fs::metadata(candidate)
                .is_ok_and(|metadata| metadata.is_file() && metadata.len() == FILE_LENGTH)
        })
        .unwrap_or(path_candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_override_is_never_silently_replaced() {
        assert_eq!(
            select(
                Some("explicit.exe".into()),
                "path.exe".into(),
                ["fallback.exe".into()]
            ),
            PathBuf::from("explicit.exe")
        );
    }

    #[test]
    fn exact_identity_rejects_length_and_digest_changes() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("opencode.exe");
        fs::write(&executable, b"proof-opencode").unwrap();
        let digest = format!("{:X}", Sha256::digest(b"proof-opencode"));
        assert!(matches_expected(&executable, 14, &digest).unwrap());
        assert!(!matches_expected(&executable, 13, &digest).unwrap());
        fs::write(&executable, b"other-opencode").unwrap();
        assert!(!matches_expected(&executable, 14, &digest).unwrap());
    }
}
