//! Shared subprocess execution with captured output and a process-group
//! timeout. Output is drained on separate threads so a runner that produces
//! more than a pipe buffer's worth of text cannot deadlock the wait.

use std::io::Read as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::Context as _;
use wait_timeout::ChildExt as _;

/// Captured result of a shell command.
pub struct Captured {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

/// Run `cmd` via `sh -c` in `dir`, capturing stdout/stderr, killing the whole
/// process group if it exceeds `timeout_secs`.
pub fn run_capture(cmd: &str, dir: &Path, timeout_secs: u64) -> anyhow::Result<Captured> {
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(cmd)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("spawn command: {cmd}"))?;

    // Drain both pipes concurrently so a full buffer can't block wait_timeout.
    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let out_handle = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(p) = out_pipe.as_mut() {
            let _ = p.read_to_string(&mut s);
        }
        s
    });
    let err_handle = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(p) = err_pipe.as_mut() {
            let _ = p.read_to_string(&mut s);
        }
        s
    });

    let (status, timed_out) = match child.wait_timeout(Duration::from_secs(timeout_secs))? {
        Some(status) => (Some(status), false),
        None => {
            kill_group(&child);
            let s = child.wait().ok();
            (s, true)
        }
    };

    let stdout = out_handle.join().unwrap_or_default();
    let stderr = err_handle.join().unwrap_or_default();
    Ok(Captured {
        success: status.map(|s| s.success()).unwrap_or(false) && !timed_out,
        code: status.and_then(|s| s.code()),
        stdout,
        stderr,
        timed_out,
    })
}

fn kill_group(child: &std::process::Child) {
    #[cfg(unix)]
    {
        let pid = child.id() as i32;
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    {
        // Best effort on non-unix; no stable process-group kill in std.
        let _ = child;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_stdout_and_exit() {
        let dir = tempfile::tempdir().unwrap();
        let c = run_capture("echo hello; exit 0", dir.path(), 10).unwrap();
        assert!(c.success);
        assert!(c.stdout.contains("hello"));
        assert!(!c.timed_out);
    }

    #[test]
    fn captures_failure_and_stderr() {
        let dir = tempfile::tempdir().unwrap();
        let c = run_capture("echo oops 1>&2; exit 3", dir.path(), 10).unwrap();
        assert!(!c.success);
        assert_eq!(c.code, Some(3));
        assert!(c.stderr.contains("oops"));
    }

    #[test]
    fn large_output_does_not_deadlock() {
        // > 64KiB (typical pipe buffer) must not hang the wait.
        let dir = tempfile::tempdir().unwrap();
        let c = run_capture("head -c 500000 /dev/zero | tr '\\0' 'a'", dir.path(), 30).unwrap();
        assert!(c.success);
        assert_eq!(c.stdout.len(), 500000);
    }

    #[test]
    fn timeout_kills_and_flags() {
        let dir = tempfile::tempdir().unwrap();
        let c = run_capture("sleep 30", dir.path(), 1).unwrap();
        assert!(c.timed_out);
        assert!(!c.success);
    }
}
