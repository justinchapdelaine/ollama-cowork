#[cfg(windows)]
mod windows;

use ollama_cowork_core::JobCleanup;
use std::process::{Child, Command};

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
        })
    }

    pub fn id(&self) -> u32 {
        self.child.id()
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
        Ok(())
    }
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
