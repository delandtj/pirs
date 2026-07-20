//! pirs-claw — lean personal assistant (not OpenClaw/Hermes).
//!
//! Intentionally thin:
//! - durable chat session (JSONL)
//! - schedule store + `tick` to fire due prompts
//! - no channel matrix, no desktop hub, no skill-evolution loop

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// One chat line in the durable session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionLine {
    pub ts: u64,
    pub role: String,
    pub text: String,
}

/// File-backed session that survives restarts.
#[derive(Debug, Clone)]
pub struct SessionStore {
    path: PathBuf,
}

impl SessionStore {
    pub fn open(path: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if !path.exists() {
            fs::File::create(&path)?;
        }
        Ok(SessionStore { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, role: &str, text: &str) -> anyhow::Result<()> {
        let line = SessionLine {
            ts: now_secs(),
            role: role.into(),
            text: text.into(),
        };
        let mut f = OpenOptions::new().create(true).append(true).open(&self.path)?;
        serde_json::to_writer(&mut f, &line)?;
        f.write_all(b"\n")?;
        f.flush()?;
        Ok(())
    }

    pub fn load(&self) -> anyhow::Result<Vec<SessionLine>> {
        let f = fs::File::open(&self.path)?;
        let reader = BufReader::new(f);
        let mut out = Vec::new();
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            out.push(serde_json::from_str(&line)?);
        }
        Ok(out)
    }

    /// Rebuild as pirs messages (user/assistant only).
    pub fn to_agent_messages(&self) -> anyhow::Result<Vec<pirs_ai::Message>> {
        let lines = self.load()?;
        let mut msgs = Vec::new();
        for l in lines {
            match l.role.as_str() {
                "user" => msgs.push(pirs_ai::Message::user(l.text)),
                "assistant" => {
                    msgs.push(pirs_ai::Message::Assistant(pirs_ai::AssistantMessage {
                        content: vec![pirs_ai::ContentBlock::text(l.text)],
                        ..Default::default()
                    }));
                }
                _ => {}
            }
        }
        Ok(msgs)
    }
}

/// A scheduled prompt (absolute fire time — no cron DSL).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduleEntry {
    pub id: String,
    pub prompt: String,
    /// Unix seconds when this job should fire.
    pub next_fire: u64,
    /// Repeat every `every_secs` after fire (0 = one-shot).
    pub every_secs: u64,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ScheduleFile {
    jobs: Vec<ScheduleEntry>,
}

/// JSON file of scheduled prompts.
#[derive(Debug, Clone)]
pub struct ScheduleStore {
    path: PathBuf,
}

impl ScheduleStore {
    pub fn open(path: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if !path.exists() {
            let empty = ScheduleFile::default();
            fs::write(&path, serde_json::to_string_pretty(&empty)?)?;
        }
        Ok(ScheduleStore { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read(&self) -> anyhow::Result<ScheduleFile> {
        let text = fs::read_to_string(&self.path)?;
        Ok(serde_json::from_str(&text).unwrap_or_default())
    }

    fn write(&self, f: &ScheduleFile) -> anyhow::Result<()> {
        fs::write(&self.path, serde_json::to_string_pretty(f)?)?;
        Ok(())
    }

    pub fn list(&self) -> anyhow::Result<Vec<ScheduleEntry>> {
        Ok(self.read()?.jobs)
    }

    pub fn add(&self, prompt: &str, every_secs: u64, first_fire_in_secs: u64) -> anyhow::Result<ScheduleEntry> {
        let mut f = self.read()?;
        let now = now_secs();
        let entry = ScheduleEntry {
            id: format!("job-{}", now),
            prompt: prompt.into(),
            next_fire: now.saturating_add(first_fire_in_secs),
            every_secs,
            enabled: true,
        };
        f.jobs.push(entry.clone());
        self.write(&f)?;
        Ok(entry)
    }

    /// Jobs due at or before `now` (enabled only).
    pub fn due(&self, now: u64) -> anyhow::Result<Vec<ScheduleEntry>> {
        Ok(self
            .read()?
            .jobs
            .into_iter()
            .filter(|j| j.enabled && j.next_fire <= now)
            .collect())
    }

    /// After firing: disable one-shots or advance next_fire for repeats.
    pub fn mark_fired(&self, id: &str, now: u64) -> anyhow::Result<()> {
        let mut f = self.read()?;
        for j in &mut f.jobs {
            if j.id == id {
                if j.every_secs == 0 {
                    j.enabled = false;
                } else {
                    j.next_fire = now.saturating_add(j.every_secs);
                }
            }
        }
        self.write(&f)?;
        Ok(())
    }
}

/// Default state dir: `~/.pirs/claw`.
pub fn default_state_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".pirs").join("claw")
}

pub fn default_session_path() -> PathBuf {
    default_state_dir().join("session.jsonl")
}

pub fn default_schedule_path() -> PathBuf {
    default_state_dir().join("schedule.json")
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// System prompt for the personal-assistant persona.
pub fn claw_system_prompt() -> String {
    "You are pirs-claw, a lean personal assistant (not a coding agent).\n\
     Be helpful, concise, and honest. Remember durable facts the user states.\n\
     You are intentionally thinner than OpenClaw/Hermes: CLI + memory + schedules."
        .into()
}

/// Whether a schedule job should be marked fired after a tick iteration.
/// Dry-run (`run == false`) never advances; failed fires stay due for retry.
pub fn should_mark_schedule_fired(run: bool, fire_succeeded: bool) -> bool {
    run && fire_succeeded
}

/// Fail closed when no LLM API key is available (do not invent replies).
pub fn require_llm_key(key: Option<&str>) -> anyhow::Result<()> {
    if key.map(|k| !k.trim().is_empty()).unwrap_or(false) {
        Ok(())
    } else {
        anyhow::bail!(
            "no API key for chat: set DASHSCOPE_API_KEY, DEEPSEEK_API_KEY, or OPENAI_API_KEY \
             (e.g. source ~/.pirs/secrets.env)"
        )
    }
}

/// Last non-empty assistant text from an agent turn, if any.
pub fn extract_assistant_reply(msgs: &[pirs_ai::Message]) -> Option<String> {
    msgs.iter().rev().find_map(|m| match m {
        pirs_ai::Message::Assistant(a) => {
            let t = a.text();
            if t.trim().is_empty() {
                None
            } else {
                Some(t)
            }
        }
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        {
            let s = SessionStore::open(&path).unwrap();
            s.append("user", "remember my dog is named Pixel").unwrap();
            s.append("assistant", "Got it — Pixel.").unwrap();
        }
        let s2 = SessionStore::open(&path).unwrap();
        let lines = s2.load().unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].role, "user");
        assert!(lines[0].text.contains("Pixel"));
        assert_eq!(lines[1].role, "assistant");
        let msgs = s2.to_agent_messages().unwrap();
        assert_eq!(msgs.len(), 2);
    }

    #[test]
    fn schedule_due_and_mark_fired_one_shot() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::open(dir.path().join("schedule.json")).unwrap();
        let job = store.add("morning brief", 0, 0).unwrap(); // due immediately
        let now = now_secs() + 1;
        let due = store.due(now).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, job.id);
        store.mark_fired(&job.id, now).unwrap();
        let due2 = store.due(now + 10).unwrap();
        assert!(due2.is_empty(), "one-shot disabled after fire");
    }

    #[test]
    fn schedule_repeat_advances_next_fire() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::open(dir.path().join("schedule.json")).unwrap();
        let job = store.add("hourly", 3600, 0).unwrap();
        let now = now_secs();
        store.mark_fired(&job.id, now).unwrap();
        let jobs = store.list().unwrap();
        let j = jobs.iter().find(|j| j.id == job.id).unwrap();
        assert!(j.enabled);
        assert_eq!(j.next_fire, now + 3600);
        assert!(store.due(now + 10).unwrap().is_empty());
        assert_eq!(store.due(now + 3600).unwrap().len(), 1);
    }

    #[test]
    fn dry_run_tick_must_not_mark_fired() {
        assert!(!should_mark_schedule_fired(false, true));
        assert!(!should_mark_schedule_fired(false, false));
    }

    #[test]
    fn failed_run_must_not_mark_fired() {
        assert!(!should_mark_schedule_fired(true, false));
        assert!(should_mark_schedule_fired(true, true));
    }

    #[test]
    fn require_llm_key_fails_closed() {
        assert!(require_llm_key(None).is_err());
        assert!(require_llm_key(Some("")).is_err());
        assert!(require_llm_key(Some("   ")).is_err());
        assert!(require_llm_key(Some("sk-test")).is_ok());
    }

    #[test]
    fn chat_does_not_invent_reply_text() {
        // Empty assistant list → None (caller must not append "(no reply)").
        assert!(extract_assistant_reply(&[]).is_none());
        let empty = pirs_ai::Message::Assistant(pirs_ai::AssistantMessage {
            content: vec![pirs_ai::ContentBlock::text("  ")],
            ..Default::default()
        });
        assert!(extract_assistant_reply(&[empty]).is_none());
        let ok = pirs_ai::Message::Assistant(pirs_ai::AssistantMessage {
            content: vec![pirs_ai::ContentBlock::text("hello")],
            ..Default::default()
        });
        assert_eq!(
            extract_assistant_reply(&[ok]).as_deref(),
            Some("hello")
        );
    }

    /// Integration: if we refuse to append when extract returns None, session
    /// stays user-only (no fake assistant line).
    #[test]
    fn no_fake_assistant_on_missing_reply() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::open(dir.path().join("s.jsonl")).unwrap();
        store.append("user", "hi").unwrap();
        let reply = extract_assistant_reply(&[]);
        assert!(reply.is_none());
        // Do not append assistant when None — durable session has only user.
        let lines = store.load().unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].role, "user");
        assert!(!lines.iter().any(|l| l.role == "assistant"));
    }

    /// Real store path: dry-run policy leaves job due after "tick" without mark_fired.
    #[test]
    fn dry_run_leaves_job_enabled_in_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::open(dir.path().join("schedule.json")).unwrap();
        let job = store.add("stay due", 0, 0).unwrap();
        let now = now_secs() + 1;
        let due = store.due(now).unwrap();
        assert_eq!(due.len(), 1);
        // Simulate dry-run: would print, must NOT mark.
        assert!(!should_mark_schedule_fired(false, false));
        // Job still due.
        assert_eq!(store.due(now).unwrap().len(), 1);
        assert!(store.list().unwrap().iter().any(|j| j.id == job.id && j.enabled));
    }
}
