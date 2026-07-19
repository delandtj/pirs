//! Flight recorder — a complete, append-only event trace of a run.
//!
//! The goal is *total recall*: every LLM turn (full messages), every tool call
//! (full args + output), every retry, phase, and outcome, each stamped with a
//! monotonic sequence number and a millisecond offset from the run's start. The
//! trace is written as JSONL (one JSON object per line, flushed immediately) so a
//! crashed run keeps everything up to the crash, and so it is trivially queryable
//! with `jq`.
//!
//! Two kinds of records share the stream:
//! - [`Recorder::agent_event`] serializes a whole [`AgentEvent`] verbatim — this
//!   is the full-fidelity capture (messages, tool args/results, turn boundaries).
//! - [`Recorder::event`] writes an arbitrary structured record, for the layers
//!   above the agent loop (harness phases, attempts, verdicts, outcomes).
//!
//! Timestamps come from a monotonic [`Instant`] taken at construction; wall-clock
//! is recorded once as `started_unix` in the header so the trace can be placed on
//! an absolute timeline without depending on the clock per event.

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::{Map, Value};

use crate::events::AgentEvent;

/// A thread-safe JSONL event recorder. Cheap to clone (it is meant to be held as
/// `Arc<Recorder>` and shared into every event source — the agent loop's emit
/// hook, the driver, the harness).
pub struct Recorder {
    writer: Mutex<Box<dyn Write + Send>>,
    start: Instant,
    seq: AtomicU64,
}

impl Recorder {
    /// Record to a JSONL file, creating/truncating it. Writes a `run.start`
    /// header line carrying the run id and absolute start time.
    pub fn to_file(path: &Path, run_id: &str, started_unix: u64) -> std::io::Result<Arc<Self>> {
        let file = std::fs::File::create(path)?;
        Ok(Self::new(Box::new(file), run_id, started_unix))
    }

    /// Build over any writer (used for in-memory capture in tests).
    pub fn new(writer: Box<dyn Write + Send>, run_id: &str, started_unix: u64) -> Arc<Self> {
        let rec = Arc::new(Recorder {
            writer: Mutex::new(writer),
            start: Instant::now(),
            seq: AtomicU64::new(0),
        });
        rec.event(
            "run.start",
            serde_json::json!({ "run_id": run_id, "started_unix": started_unix }),
        );
        rec
    }

    /// An in-memory recorder plus the shared buffer it writes to (for tests and
    /// callers that want the trace as bytes rather than a file).
    pub fn in_memory(run_id: &str) -> (Arc<Self>, Arc<Mutex<Vec<u8>>>) {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let rec = Self::new(Box::new(SharedBuf(Arc::clone(&buf))), run_id, 0);
        (rec, buf)
    }

    /// Stamp `seq` + `t_ms` onto `obj` and write it as one JSONL line, flushing so
    /// the record survives a crash.
    fn write_obj(&self, mut obj: Map<String, Value>) {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let t_ms = self.start.elapsed().as_millis() as u64;
        obj.insert("seq".into(), seq.into());
        obj.insert("t_ms".into(), t_ms.into());
        if let Ok(mut w) = self.writer.lock() {
            let line = Value::Object(obj);
            let _ = serde_json::to_writer(&mut *w, &line);
            let _ = w.write_all(b"\n");
            let _ = w.flush();
        }
    }

    /// Write an arbitrary record: `kind` plus the (object) fields of `data`.
    pub fn event(&self, kind: &str, data: Value) {
        let mut obj = Map::new();
        obj.insert("kind".into(), Value::String(kind.to_string()));
        if let Value::Object(fields) = data {
            for (k, v) in fields {
                obj.insert(k, v);
            }
        }
        self.write_obj(obj);
    }

    /// Capture a full [`AgentEvent`] verbatim, tagged with the phase it occurred
    /// in. This is the full-fidelity record: messages, tool args/results, turns.
    pub fn agent_event(&self, phase: &str, ev: &AgentEvent) {
        let mut obj = Map::new();
        obj.insert("kind".into(), Value::String("agent".into()));
        obj.insert("phase".into(), Value::String(phase.to_string()));
        obj.insert(
            "event".into(),
            serde_json::to_value(ev).unwrap_or(Value::Null),
        );
        self.write_obj(obj);
    }
}

/// A `Write` over a shared byte buffer, so an in-memory recorder's bytes can be
/// read back after recording.
struct SharedBuf(Arc<Mutex<Vec<u8>>>);
impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(buf: &Arc<Mutex<Vec<u8>>>) -> Vec<Value> {
        let bytes = buf.lock().unwrap().clone();
        String::from_utf8(bytes)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .collect()
    }

    #[test]
    fn header_then_events_get_monotonic_seq() {
        let (rec, buf) = Recorder::in_memory("run-1");
        rec.event("instance.start", serde_json::json!({ "id": "abc" }));
        rec.event("instance.end", serde_json::json!({ "outcome": "solved" }));
        let ls = lines(&buf);
        assert_eq!(ls.len(), 3); // header + 2
        assert_eq!(ls[0]["kind"], "run.start");
        assert_eq!(ls[0]["run_id"], "run-1");
        assert_eq!(ls[1]["kind"], "instance.start");
        assert_eq!(ls[1]["id"], "abc");
        // seq strictly increases and t_ms is present on every line.
        for (i, l) in ls.iter().enumerate() {
            assert_eq!(l["seq"], i as u64);
            assert!(l["t_ms"].is_u64());
        }
    }

    #[test]
    fn agent_events_are_captured_verbatim() {
        let (rec, buf) = Recorder::in_memory("r");
        rec.agent_event(
            "plan-exec#0",
            &AgentEvent::ToolExecutionStart {
                tool_call_id: "t1".into(),
                tool_name: "read".into(),
                args: serde_json::json!({ "path": "a.py" }),
            },
        );
        let ls = lines(&buf);
        let ev = &ls[1];
        assert_eq!(ev["kind"], "agent");
        assert_eq!(ev["phase"], "plan-exec#0");
        // The whole AgentEvent is embedded, internally tagged by `type`.
        assert_eq!(ev["event"]["type"], "tool_execution_start");
        assert_eq!(ev["event"]["toolName"], "read");
        assert_eq!(ev["event"]["args"]["path"], "a.py");
    }

    #[test]
    fn concurrent_recorders_do_not_interleave_partial_lines() {
        use std::thread;
        let (rec, buf) = Recorder::in_memory("r");
        let mut handles = Vec::new();
        for n in 0..8 {
            let r = Arc::clone(&rec);
            handles.push(thread::spawn(move || {
                for i in 0..50 {
                    r.event("tick", serde_json::json!({ "worker": n, "i": i }));
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        // Every line must parse as complete JSON (no torn writes), header + 400.
        let ls = lines(&buf);
        assert_eq!(ls.len(), 1 + 8 * 50);
    }
}
