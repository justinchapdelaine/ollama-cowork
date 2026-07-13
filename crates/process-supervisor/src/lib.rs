#[cfg(windows)]
mod windows;

use ollama_cowork_core::JobCleanup;
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    thread::{self, JoinHandle},
};

#[cfg(windows)]
use windows::ProcessTree;

#[cfg(not(windows))]
struct ProcessTree;

#[cfg(not(windows))]
impl ProcessTree {
    fn prepare(_: &mut Command) {}
    fn attach(_: &Child) -> std::io::Result<Self> {
        Ok(Self)
    }
    fn terminate(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub struct ManagedChild {
    child: Child,
    tree: ProcessTree,
    terminated: bool,
    captures: Vec<JoinHandle<std::io::Result<()>>>,
}

impl ManagedChild {
    pub fn spawn(command: &mut Command) -> std::io::Result<Self> {
        ProcessTree::prepare(command);
        let mut child = command.spawn()?;
        let tree = match ProcessTree::attach(&child) {
            Ok(tree) => tree,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        Ok(Self {
            child,
            tree,
            terminated: false,
            captures: Vec::new(),
        })
    }

    pub fn spawn_with_bounded_logs(
        command: &mut Command,
        stdout_path: &Path,
        stderr_path: &Path,
        limit_bytes: u64,
    ) -> std::io::Result<Self> {
        if limit_bytes == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "log limit must be positive",
            ));
        }
        let stdout_file = create_new_log(stdout_path)?;
        let stderr_file = create_new_log(stderr_path)?;
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut managed = Self::spawn(command)?;
        let stdout = managed.child.stdout.take().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "child stdout is unavailable",
            )
        })?;
        let stderr = managed.child.stderr.take().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "child stderr is unavailable",
            )
        })?;
        managed.captures = vec![
            spawn_bounded_capture(stdout, stdout_file, limit_bytes),
            spawn_bounded_capture(stderr, stderr_file, limit_bytes),
        ];
        Ok(managed)
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// Transfers one bootstrap payload to the child and closes stdin so the
    /// secret material is never persisted in the child's working directory.
    pub fn write_stdin_and_close(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let mut stdin = self.child.stdin.take().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "child stdin is unavailable")
        })?;
        stdin.write_all(bytes)?;
        stdin.flush()
    }

    pub fn stop(&mut self) -> std::io::Result<()> {
        if self.terminated {
            return Ok(());
        }
        self.tree.terminate()?;
        if self.child.try_wait()?.is_none() {
            #[cfg(not(windows))]
            self.child.kill()?;
            self.child.wait()?;
        }
        self.terminated = true;
        for capture in self.captures.drain(..) {
            capture
                .join()
                .map_err(|_| std::io::Error::other("log capture thread panicked"))??;
        }
        Ok(())
    }
}

fn create_new_log(path: &Path) -> std::io::Result<std::fs::File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

fn spawn_bounded_capture(
    mut source: impl Read + Send + 'static,
    mut destination: std::fs::File,
    limit_bytes: u64,
) -> JoinHandle<std::io::Result<()>> {
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        let mut remaining = limit_bytes;
        loop {
            let read = source.read(&mut buffer)?;
            if read == 0 {
                return Ok(());
            }
            let writable = read.min(remaining as usize);
            if writable > 0 {
                destination.write_all(&buffer[..writable])?;
                remaining -= writable as u64;
            }
        }
    })
}

impl JobCleanup for ManagedChild {
    fn terminate(&mut self) -> Result<(), String> {
        self.stop().map_err(|error| error.to_string())
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(windows)]
pub fn loopback_listener_owner(port: u16) -> std::io::Result<Option<u32>> {
    use windows_sys::Win32::{
        Foundation::{ERROR_INSUFFICIENT_BUFFER, NO_ERROR},
        NetworkManagement::IpHelper::{GetExtendedTcpTable, TCP_TABLE_OWNER_PID_LISTENER},
        Networking::WinSock::AF_INET,
    };
    let mut size = 0_u32;
    let first = unsafe {
        GetExtendedTcpTable(
            std::ptr::null_mut(),
            &mut size,
            0,
            AF_INET as u32,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        )
    };
    if first != ERROR_INSUFFICIENT_BUFFER {
        return Err(std::io::Error::from_raw_os_error(first as i32));
    }
    let mut table = vec![0_u8; size as usize];
    let status = unsafe {
        GetExtendedTcpTable(
            table.as_mut_ptr().cast(),
            &mut size,
            0,
            AF_INET as u32,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        )
    };
    if status != NO_ERROR {
        return Err(std::io::Error::from_raw_os_error(status as i32));
    }
    let count = u32::from_ne_bytes(table[0..4].try_into().unwrap()) as usize;
    const ROW_BYTES: usize = 24;
    for row in table[4..].chunks_exact(ROW_BYTES).take(count) {
        let local_address = u32::from_ne_bytes(row[4..8].try_into().unwrap());
        let local_port = u16::from_be_bytes(row[8..10].try_into().unwrap());
        let owner = u32::from_ne_bytes(row[20..24].try_into().unwrap());
        if local_address == u32::from_ne_bytes([127, 0, 0, 1]) && local_port == port {
            return Ok(Some(owner));
        }
    }
    Ok(None)
}

#[cfg(windows)]
pub fn loopback_listener_owned_by(port: u16, process_id: u32) -> std::io::Result<bool> {
    let Some(owner) = loopback_listener_owner(port)? else {
        return Ok(false);
    };
    Ok(owner == process_id || windows::is_process_descendant(owner, process_id)?)
}

#[cfg(not(windows))]
pub fn loopback_listener_owned_by(_port: u16, _process_id: u32) -> std::io::Result<bool> {
    Ok(true)
}

#[cfg(not(windows))]
pub fn loopback_listener_owner(_port: u16) -> std::io::Result<Option<u32>> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_child_stops_idempotently() {
        let mut command = if cfg!(windows) {
            let mut command = Command::new("cmd.exe");
            command.args(["/C", "ping -n 30 127.0.0.1 >nul"]);
            command
        } else {
            let mut command = Command::new("sleep");
            command.arg("30");
            command
        };
        let mut child = ManagedChild::spawn(&mut command).unwrap();
        assert_ne!(child.id(), 0);
        child.stop().unwrap();
        child.stop().unwrap();
    }

    #[test]
    fn bounded_log_capture_never_exceeds_its_policy_limit() {
        let temp = tempfile::tempdir().unwrap();
        let stdout = temp.path().join("stdout.log");
        let stderr = temp.path().join("stderr.log");
        let mut command = if cfg!(windows) {
            let mut command = Command::new("powershell.exe");
            command.args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "$text='x'*65536; [Console]::Out.Write($text); [Console]::Error.Write($text)",
            ]);
            command
        } else {
            let mut command = Command::new("sh");
            command.args(["-c", "head -c 65536 /dev/zero; head -c 65536 /dev/zero >&2"]);
            command
        };
        let mut child =
            ManagedChild::spawn_with_bounded_logs(&mut command, &stdout, &stderr, 4096).unwrap();
        child.stop().unwrap();
        assert!(std::fs::metadata(stdout).unwrap().len() <= 4096);
        assert!(std::fs::metadata(stderr).unwrap().len() <= 4096);
    }

    #[cfg(windows)]
    #[test]
    fn windows_listener_attestation_recognizes_current_process() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(loopback_listener_owned_by(port, std::process::id()).unwrap());
        assert!(!loopback_listener_owned_by(port, std::process::id() + 1).unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn windows_job_terminates_descendant_processes() {
        use std::{
            fs, thread,
            time::{Duration, Instant},
        };
        use windows_sys::Win32::{
            Foundation::{CloseHandle, WAIT_OBJECT_0},
            System::Threading::{
                OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess,
                WaitForSingleObject,
            },
        };

        let temp = tempfile::tempdir().unwrap();
        let pid_file = temp.path().join("descendant.pid");
        let mut command = Command::new("powershell.exe");
        command
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "$p=Start-Process powershell.exe -ArgumentList @('-NoLogo','-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 60') -PassThru; [IO.File]::WriteAllText($env:PID_FILE,[string]$p.Id); Wait-Process -Id $p.Id",
            ])
            .env("PID_FILE", &pid_file);
        let mut child = ManagedChild::spawn(&mut command).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !pid_file.is_file() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(25));
        }
        let descendant_id: u32 = fs::read_to_string(&pid_file)
            .expect("descendant pid was not published")
            .trim()
            .parse()
            .unwrap();
        child.stop().unwrap();

        let process =
            unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, 0, descendant_id) };
        if process.is_null() {
            return;
        }
        let exited = unsafe { WaitForSingleObject(process, 2_000) } == WAIT_OBJECT_0;
        if !exited {
            unsafe {
                TerminateProcess(process, 1);
            }
        }
        unsafe {
            CloseHandle(process);
        }
        assert!(exited, "descendant survived managed process cleanup");
    }
}
