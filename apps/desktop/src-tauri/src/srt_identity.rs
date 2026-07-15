use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::Path};

/// Published npm package validated by the Windows Spike 001 enforcement matrix.
pub const PACKAGE_VERSION: &str = "0.0.65";
/// The bundled Windows helper has its own protocol/version identity.
pub const HELPER_VERSION: &str = "srt-win 0.0.1";
pub const SHA256: &str = "17A63AA8C010662B3E723F75D13D8672C69BEECA8D072F4B2DCE7484E850023A";
pub const FILE_LENGTH: u64 = 2_656_768;

pub fn matches(path: &Path) -> Result<bool, String> {
    matches_expected(path, FILE_LENGTH, SHA256)
}

pub fn matches_expected(
    path: &Path,
    expected_length: u64,
    expected_sha256: &str,
) -> Result<bool, String> {
    let mut file =
        fs::File::open(path).map_err(|error| format!("could not read SRT helper: {error}"))?;
    if file
        .metadata()
        .map_err(|error| format!("could not inspect SRT helper: {error}"))?
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
            .map_err(|error| format!("could not hash SRT helper: {error}"))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:X}", digest.finalize()) == expected_sha256)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_identity_rejects_length_and_digest_changes() {
        let root = tempfile::tempdir().unwrap();
        let helper = root.path().join("srt-win.exe");
        fs::write(&helper, b"proof-helper").unwrap();
        let digest = format!("{:X}", Sha256::digest(b"proof-helper"));
        assert!(matches_expected(&helper, 12, &digest).unwrap());
        assert!(!matches_expected(&helper, 11, &digest).unwrap());
        fs::write(&helper, b"other-helper").unwrap();
        assert!(!matches_expected(&helper, 12, &digest).unwrap());
    }
}
