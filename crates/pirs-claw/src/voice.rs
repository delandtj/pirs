//! Voice memo transcription hook (Hermes gap: voice → text).
//!
//! Does not embed a model. Uses external CLI if present:
//! - `whisper` / `whisper-cpp` / `faster-whisper`
//! - or `PIRS_CLAW_TRANSCRIBE_CMD` template with `{path}` placeholder
//!
//! Returns None when no transcriber is available (caller keeps original).

use std::path::Path;
use std::process::Command;

/// Try to transcribe an audio file to text.
pub fn transcribe_audio(path: &Path) -> anyhow::Result<Option<String>> {
    if !path.is_file() {
        anyhow::bail!("audio file not found: {}", path.display());
    }
    if let Ok(tmpl) = std::env::var("PIRS_CLAW_TRANSCRIBE_CMD") {
        let cmd = tmpl.replace("{path}", &path.display().to_string());
        let out = Command::new("sh").arg("-c").arg(&cmd).output()?;
        if out.status.success() {
            let t = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !t.is_empty() {
                return Ok(Some(t));
            }
        }
        return Ok(None);
    }
    for bin in ["whisper", "whisper-cpp", "faster-whisper"] {
        if which(bin).is_none() {
            continue;
        }
        let out = Command::new(bin)
            .arg(path.as_os_str())
            .arg("--output_format")
            .arg("txt")
            .output();
        if let Ok(out) = out {
            if out.status.success() {
                let t = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !t.is_empty() {
                    return Ok(Some(t));
                }
            }
        }
    }
    Ok(None)
}

fn which(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_errors() {
        let err = transcribe_audio(Path::new("/no/such/audio.ogg")).unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn no_transcriber_returns_none_for_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.ogg");
        std::fs::write(&p, b"").unwrap();
        // Without whisper on PATH and without custom cmd, expect Ok(None).
        std::env::remove_var("PIRS_CLAW_TRANSCRIBE_CMD");
        let r = transcribe_audio(&p).unwrap();
        assert!(r.is_none() || r.as_ref().map(|s| s.is_empty()).unwrap_or(true));
    }
}
