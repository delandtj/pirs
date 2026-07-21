//! Single-instance lock (Telegram getUpdates is exclusive per bot token).

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Held lock; releasing the process or dropping unlocks (flock).
#[derive(Debug)]
pub struct InstanceLock {
    _file: File,
    path: PathBuf,
}

impl InstanceLock {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Try to acquire an exclusive non-blocking lock under `state_dir/locks/{name}.lock`.
pub fn try_acquire(state_dir: &Path, name: &str) -> anyhow::Result<InstanceLock> {
    let dir = state_dir.join("locks");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{name}.lock"));
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(true)
        .open(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let fd = file.as_raw_fd();
        let rc = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
        if rc != 0 {
            anyhow::bail!(
                "another pirs-claw instance already holds the {name:?} lock ({})\n\
                 Telegram getUpdates allows only one long-poll per bot token.\n\
                 Stop the other process, or remove the stale lock if nothing is running.",
                path.display()
            );
        }
    }
    let pid = std::process::id();
    let _ = writeln!(file, "pid={pid}\nname={name}\n");
    let _ = file.flush();
    Ok(InstanceLock { _file: file, path })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_lock_fails() {
        let dir = tempfile::tempdir().unwrap();
        let a = try_acquire(dir.path(), "telegram").unwrap();
        let err = try_acquire(dir.path(), "telegram").unwrap_err().to_string();
        assert!(err.contains("already holds") || err.contains("lock"), "{err}");
        drop(a);
        // After drop, can re-acquire
        let _b = try_acquire(dir.path(), "telegram").unwrap();
    }
}
