use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

pub fn canonical_input(path: &Path) -> Result<PathBuf> {
    let path =
        fs::canonicalize(path).with_context(|| format!("resolve input {}", path.display()))?;
    if path
        .extension()
        .and_then(|v| v.to_str())
        .map(|v| v.eq_ignore_ascii_case("docx"))
        != Some(true)
    {
        bail!("input must have .docx extension")
    }
    Ok(path)
}

pub fn validate_new_output(input: &Path, output: &Path) -> Result<PathBuf> {
    if output
        .extension()
        .and_then(|v| v.to_str())
        .map(|v| v.eq_ignore_ascii_case("docx"))
        != Some(true)
    {
        bail!("output must have .docx extension")
    }
    if output.exists() {
        bail!("output already exists; overwrite is forbidden")
    }
    let parent = output.parent().context("output has no parent")?;
    let parent = fs::canonicalize(parent)
        .with_context(|| format!("resolve output parent {}", parent.display()))?;
    let name = output.file_name().context("output has no file name")?;
    let resolved = parent.join(name);
    if resolved == input {
        bail!("output must be a revised copy, not the source")
    }
    Ok(resolved)
}

pub fn sha256(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
