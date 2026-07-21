//! Terminal backends (Hermes gap: local / Docker / SSH).
//!
//! Maps to `pirs_tools::sandbox` via `PIRS_SANDBOX`:
//! - `local` (default) — host bash
//! - `docker` | `docker:<image>` | `docker@<container>` — Docker exec
//! - `ssh:user@host` | `ssh:user@host:port` — remote bash over SSH
//!
//! Explicitly **not** covering Hermes Modal / Daytona / Singularity.

use std::env;

/// Apply exec backend for this process (bash tool reads `PIRS_SANDBOX`).
pub fn apply_exec_backend(spec: &str) -> anyhow::Result<String> {
    let s = spec.trim();
    if s.is_empty() || s == "local" {
        env::remove_var("PIRS_SANDBOX");
        return Ok("local".into());
    }
    if s == "docker" {
        env::set_var("PIRS_SANDBOX", "docker");
        return Ok("docker".into());
    }
    if let Some(rest) = s.strip_prefix("docker@") {
        // Existing container (pirs_tools uses docker:<container>).
        env::set_var("PIRS_SANDBOX", format!("docker:{rest}"));
        return Ok(format!("docker@{rest}"));
    }
    if let Some(rest) = s.strip_prefix("docker:") {
        // Image for docker run --rm (via PIRS_SANDBOX_IMAGE).
        env::set_var("PIRS_SANDBOX", "docker");
        env::set_var("PIRS_SANDBOX_IMAGE", rest);
        return Ok(format!("docker:{rest}"));
    }
    if let Some(target) = s.strip_prefix("ssh:") {
        if target.is_empty() {
            anyhow::bail!("--exec ssh: requires user@host");
        }
        env::set_var("PIRS_SANDBOX", format!("ssh:{target}"));
        return Ok(format!("ssh:{target}"));
    }
    if s.eq_ignore_ascii_case("modal")
        || s.eq_ignore_ascii_case("daytona")
        || s.eq_ignore_ascii_case("singularity")
    {
        anyhow::bail!(
            "exec backend {s:?} is intentionally unsupported (Hermes gap exclusion). \
             Use local, docker, docker:<image>, docker@<container>, or ssh:user@host"
        );
    }
    anyhow::bail!(
        "unknown --exec {s:?}; expected local | docker | docker:<image> | docker@<container> | ssh:user@host"
    )
}

pub fn describe_active() -> String {
    env::var("PIRS_SANDBOX").unwrap_or_else(|_| "local".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_backend_specs_roundtrip() {
        // Single test mutates process env (avoid parallel races).
        assert_eq!(apply_exec_backend("docker").unwrap(), "docker");
        assert_eq!(env::var("PIRS_SANDBOX").unwrap(), "docker");
        assert_eq!(
            apply_exec_backend("docker:ubuntu:22.04").unwrap(),
            "docker:ubuntu:22.04"
        );
        assert_eq!(env::var("PIRS_SANDBOX").unwrap(), "docker");
        assert_eq!(env::var("PIRS_SANDBOX_IMAGE").unwrap(), "ubuntu:22.04");
        assert_eq!(apply_exec_backend("docker@myctr").unwrap(), "docker@myctr");
        assert_eq!(env::var("PIRS_SANDBOX").unwrap(), "docker:myctr");
        assert_eq!(
            apply_exec_backend("ssh:me@box:22").unwrap(),
            "ssh:me@box:22"
        );
        assert_eq!(env::var("PIRS_SANDBOX").unwrap(), "ssh:me@box:22");
        assert_eq!(apply_exec_backend("local").unwrap(), "local");
        assert!(env::var("PIRS_SANDBOX").is_err());
    }

    #[test]
    fn rejects_modal_daytona_singularity() {
        for b in ["modal", "daytona", "singularity", "Modal"] {
            let e = apply_exec_backend(b).unwrap_err().to_string();
            assert!(e.contains("unsupported"), "{b}: {e}");
        }
    }
}
