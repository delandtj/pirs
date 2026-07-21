//! Channel pairing / allowlist (Hermes + OpenClaw lesson: never open bots to the world).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Env var that disables pairing (dev only).
pub const ALLOW_ALL_ENV: &str = "PIRS_CLAW_ALLOW_ALL";

/// Warning printed when allow-all is active (must stay stable for tests/docs).
pub const ALLOW_ALL_WARNING: &str =
    "WARNING: PIRS_CLAW_ALLOW_ALL is set — pairing disabled; any peer can talk to this gateway (dev only)";

/// True when `PIRS_CLAW_ALLOW_ALL` is 1/true (case-insensitive).
pub fn allow_all_enabled() -> bool {
    std::env::var(ALLOW_ALL_ENV)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Emit dangerous-dev warning to stderr if allow-all is on. Returns whether warned.
pub fn warn_if_allow_all() -> bool {
    if allow_all_enabled() {
        eprintln!("[pirs-claw] {ALLOW_ALL_WARNING}");
        true
    } else {
        false
    }
}

/// Allowlist file: one peer id per line (`chat_id`, Discord user id, …).
/// Lines starting with `#` ignored. Empty file / missing = **deny all** for
/// non-CLI channels (fail closed).
#[derive(Debug, Clone, Default)]
pub struct PairingAllowlist {
    peers: HashSet<String>,
    /// When true, any peer is allowed (dev only). Set via `PIRS_CLAW_ALLOW_ALL=1`.
    allow_all: bool,
}

impl PairingAllowlist {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let allow_all = allow_all_enabled();
        let mut peers = HashSet::new();
        if path.is_file() {
            let text = fs::read_to_string(path)?;
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                peers.insert(line.to_string());
            }
        }
        Ok(PairingAllowlist { peers, allow_all })
    }

    pub fn default_path(state_dir: &Path) -> PathBuf {
        state_dir.join("allowlist.txt")
    }

    pub fn is_allowed(&self, peer_id: &str) -> bool {
        if self.allow_all {
            return true;
        }
        self.peers.contains(peer_id)
    }

    pub fn is_empty(&self) -> bool {
        self.peers.is_empty() && !self.allow_all
    }

    pub fn len(&self) -> usize {
        self.peers.len()
    }

    pub fn allow_all(&self) -> bool {
        self.allow_all
    }

    /// Sorted list of paired peer ids (file contents, not allow_all).
    pub fn list(&self) -> Vec<String> {
        let mut v: Vec<_> = self.peers.iter().cloned().collect();
        v.sort();
        v
    }

    /// Add a peer and rewrite the allowlist file.
    pub fn add(&mut self, path: &Path, peer: &str) -> anyhow::Result<bool> {
        let peer = peer.trim();
        if peer.is_empty() {
            anyhow::bail!("peer id must be non-empty");
        }
        let inserted = self.peers.insert(peer.to_string());
        self.save(path)?;
        Ok(inserted)
    }

    /// Remove a peer and rewrite the allowlist file. Returns true if it was present.
    pub fn remove(&mut self, path: &Path, peer: &str) -> anyhow::Result<bool> {
        let removed = self.peers.remove(peer.trim());
        self.save(path)?;
        Ok(removed)
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut lines: Vec<_> = self.peers.iter().cloned().collect();
        lines.sort();
        let mut body = String::from(
            "# pirs-claw pairing allowlist — one peer id per line (chat_id / user id)\n",
        );
        for p in lines {
            body.push_str(&p);
            body.push('\n');
        }
        fs::write(path, body)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_by_default() {
        // Ensure allow-all is off for this process for the assertion.
        std::env::remove_var(ALLOW_ALL_ENV);
        let dir = tempfile::tempdir().unwrap();
        let al = PairingAllowlist::open(&dir.path().join("missing.txt")).unwrap();
        assert!(!al.is_allowed("123"));
        assert!(al.is_empty());
        assert!(!al.allow_all());
    }

    #[test]
    fn allowlisted_peer() {
        std::env::remove_var(ALLOW_ALL_ENV);
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("allowlist.txt");
        fs::write(&p, "# comment\n42\n99\n").unwrap();
        let al = PairingAllowlist::open(&p).unwrap();
        assert!(al.is_allowed("42"));
        assert!(!al.is_allowed("7"));
        assert_eq!(al.len(), 2);
    }

    #[test]
    fn allow_all_warning_text_is_stable() {
        assert!(ALLOW_ALL_WARNING.contains("PIRS_CLAW_ALLOW_ALL"));
        assert!(ALLOW_ALL_WARNING.contains("pairing disabled"));
    }

    #[test]
    fn add_list_remove_roundtrip() {
        std::env::remove_var(ALLOW_ALL_ENV);
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("allowlist.txt");
        let mut al = PairingAllowlist::open(&p).unwrap();
        assert!(al.add(&p, "111").unwrap());
        assert!(!al.add(&p, "111").unwrap()); // already present
        assert!(al.add(&p, "222").unwrap());
        assert_eq!(al.list(), vec!["111".to_string(), "222".to_string()]);
        // Reload from disk
        let al2 = PairingAllowlist::open(&p).unwrap();
        assert!(al2.is_allowed("111"));
        assert!(al2.is_allowed("222"));
        let mut al3 = PairingAllowlist::open(&p).unwrap();
        assert!(al3.remove(&p, "111").unwrap());
        assert!(!al3.is_allowed("111"));
        assert!(al3.is_allowed("222"));
    }
}
