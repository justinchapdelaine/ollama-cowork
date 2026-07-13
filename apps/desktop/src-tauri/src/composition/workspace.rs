use ollama_cowork_core::JobCleanup;
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};

#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt;

const RUN_PREFIX: &str = "run-";
const LEASE_FILE: &str = ".lease";

pub struct JobWorkspace {
    root: PathBuf,
    source: PathBuf,
    model: PathBuf,
    private_output: PathBuf,
    publish: PathBuf,
    _run_lease: File,
    terminated: bool,
}

impl JobWorkspace {
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn source(&self) -> &Path {
        &self.source
    }
    pub fn model(&self) -> &Path {
        &self.model
    }
    pub fn private_output(&self) -> &Path {
        &self.private_output
    }
    pub fn publish(&self) -> &Path {
        &self.publish
    }
}

impl JobCleanup for JobWorkspace {
    fn terminate(&mut self) -> Result<(), String> {
        if self.terminated {
            return Ok(());
        }
        if self.root.exists() {
            fs::remove_dir_all(&self.root).map_err(|error| error.to_string())?;
        }
        self.terminated = true;
        Ok(())
    }
}

impl Drop for JobWorkspace {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

pub trait JobWorkspaceFactory: Send {
    fn create(&mut self, job_id: &str, source: &Path) -> Result<JobWorkspace, String>;
}

pub struct FilesystemJobWorkspaceFactory {
    run_root: PathBuf,
    _lease: File,
    stale_cleanup: StaleRunCleanupReport,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StaleRunCleanupReport {
    pub removed: usize,
    pub retained_active: usize,
    pub retained_failed: usize,
}

impl FilesystemJobWorkspaceFactory {
    pub fn new(runs_root: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&runs_root).map_err(|error| error.to_string())?;
        let stale_cleanup = cleanup_stale_runs(&runs_root)?;
        for _ in 0..8 {
            let run_root = runs_root.join(format!("{RUN_PREFIX}{}", random_hex_128()?));
            match fs::create_dir(&run_root) {
                Ok(()) => {
                    let lease = open_exclusive_lease(&run_root.join(LEASE_FILE), true)
                        .map_err(|error| error.to_string())?;
                    return Ok(Self {
                        run_root,
                        _lease: lease,
                        stale_cleanup: stale_cleanup.clone(),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.to_string()),
            }
        }
        Err("could not allocate a unique workspace run directory".into())
    }

    pub fn run_root(&self) -> &Path {
        &self.run_root
    }

    pub fn stale_cleanup_report(&self) -> &StaleRunCleanupReport {
        &self.stale_cleanup
    }
}

impl JobWorkspaceFactory for FilesystemJobWorkspaceFactory {
    fn create(&mut self, job_id: &str, source: &Path) -> Result<JobWorkspace, String> {
        if job_id.is_empty()
            || !job_id
                .bytes()
                .all(|value| value.is_ascii_alphanumeric() || value == b'-')
        {
            return Err("job identifier is not filesystem-safe".into());
        }
        let source =
            fs::canonicalize(source).map_err(|error| format!("source is unavailable: {error}"))?;
        if source
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.eq_ignore_ascii_case("docx"))
            != Some(true)
        {
            return Err("source must be a DOCX".into());
        }
        let publish = source
            .parent()
            .ok_or_else(|| "source has no output directory".to_owned())?
            .to_path_buf();
        let run_lease = self._lease.try_clone().map_err(|error| error.to_string())?;
        let root = self.run_root.join(job_id);
        fs::create_dir(&root).map_err(|error| {
            format!("job workspace already exists or cannot be created: {error}")
        })?;
        let model = root.join("model");
        let private_output = root.join("private-output");
        if let Err(error) = fs::create_dir(&model).and_then(|_| fs::create_dir(&private_output)) {
            let _ = fs::remove_dir_all(&root);
            return Err(error.to_string());
        }
        Ok(JobWorkspace {
            root,
            source,
            model,
            private_output,
            publish,
            _run_lease: run_lease,
            terminated: false,
        })
    }
}

fn random_hex_128() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| error.to_string())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn valid_run_name(name: &str) -> bool {
    name.len() == RUN_PREFIX.len() + 32
        && name.starts_with(RUN_PREFIX)
        && name[RUN_PREFIX.len()..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(windows)]
fn open_exclusive_lease(path: &Path, create_new: bool) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(create_new)
        .share_mode(0)
        .open(path)
}

#[cfg(not(windows))]
fn open_exclusive_lease(path: &Path, create_new: bool) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(create_new)
        .open(path)
}

fn cleanup_stale_runs(runs_root: &Path) -> Result<StaleRunCleanupReport, String> {
    let mut report = StaleRunCleanupReport::default();
    for entry in fs::read_dir(runs_root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !valid_run_name(name)
            || !entry
                .file_type()
                .map_err(|error| error.to_string())?
                .is_dir()
        {
            continue;
        }
        #[cfg(windows)]
        {
            let lease_path = entry.path().join(LEASE_FILE);
            match open_exclusive_lease(&lease_path, false) {
                Ok(lease) => drop(lease),
                Err(error) if matches!(error.raw_os_error(), Some(32 | 33)) => {
                    report.retained_active += 1;
                    continue;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => {
                    report.retained_failed += 1;
                    continue;
                }
            }
            match fs::remove_dir_all(entry.path()) {
                Ok(()) => report.removed += 1,
                Err(_) => report.retained_failed += 1,
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_isolated_directories_and_removes_only_transient_state() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.docx");
        fs::write(&source, b"fixture").unwrap();
        let mut factory = FilesystemJobWorkspaceFactory::new(temp.path().join("jobs")).unwrap();
        let mut workspace = factory.create("job-1", &source).unwrap();

        assert!(workspace.model().is_dir());
        assert!(workspace.private_output().is_dir());
        assert_eq!(
            fs::canonicalize(workspace.publish()).unwrap(),
            fs::canonicalize(temp.path()).unwrap()
        );
        let root = workspace.root().to_path_buf();
        workspace.terminate().unwrap();
        assert!(!root.exists());
        assert!(source.exists());
    }

    #[test]
    fn rejects_unsafe_or_duplicate_job_directories() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.docx");
        fs::write(&source, b"fixture").unwrap();
        let mut factory = FilesystemJobWorkspaceFactory::new(temp.path().join("jobs")).unwrap();
        assert!(factory.create("../escape", &source).is_err());
        let workspace = factory.create("job-1", &source).unwrap();
        assert!(factory.create("job-1", &source).is_err());
        drop(workspace);
    }

    #[test]
    fn run_names_are_opaque_and_isolate_restarted_job_counters() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.docx");
        fs::write(&source, b"fixture").unwrap();
        let runs = temp.path().join("jobs");
        let mut first = FilesystemJobWorkspaceFactory::new(runs.clone()).unwrap();
        let first_root = first.run_root().to_path_buf();
        let first_job = first.create("job-1", &source).unwrap();
        let mut second = FilesystemJobWorkspaceFactory::new(runs).unwrap();
        let second_root = second.run_root().to_path_buf();
        let second_job = second.create("job-1", &source).unwrap();

        assert_ne!(first_root, second_root);
        assert!(valid_run_name(
            first_root.file_name().unwrap().to_str().unwrap()
        ));
        assert!(first_job.root().is_dir());
        assert!(second_job.root().is_dir());
    }

    #[cfg(windows)]
    #[test]
    fn startup_preserves_active_runs_and_removes_released_stale_runs() {
        let temp = tempfile::tempdir().unwrap();
        let runs = temp.path().join("jobs");
        let active = FilesystemJobWorkspaceFactory::new(runs.clone()).unwrap();
        let active_root = active.run_root().to_path_buf();
        let stale_root = {
            let stale = FilesystemJobWorkspaceFactory::new(runs.clone()).unwrap();
            stale.run_root().to_path_buf()
        };

        let next = FilesystemJobWorkspaceFactory::new(runs).unwrap();
        assert!(active_root.is_dir());
        assert!(!stale_root.exists());
        assert!(next.run_root().is_dir());
        assert_eq!(
            next.stale_cleanup_report(),
            &StaleRunCleanupReport {
                removed: 1,
                retained_active: 1,
                retained_failed: 0,
            }
        );
    }

    #[cfg(windows)]
    #[test]
    fn active_job_keeps_run_lease_after_factory_is_dropped() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.docx");
        fs::write(&source, b"fixture").unwrap();
        let runs = temp.path().join("jobs");
        let mut factory = FilesystemJobWorkspaceFactory::new(runs.clone()).unwrap();
        let run_root = factory.run_root().to_path_buf();
        let workspace = factory.create("job-1", &source).unwrap();
        drop(factory);

        let _next = FilesystemJobWorkspaceFactory::new(runs).unwrap();
        assert!(run_root.is_dir());
        assert!(workspace.root().is_dir());
    }
}
