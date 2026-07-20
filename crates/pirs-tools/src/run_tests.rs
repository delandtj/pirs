//! `run_tests` — a compound tool that collapses "figure out how this project runs
//! its tests" and "run them" into a single call.
//!
//! An agent without it spends turns probing: `ls`, read `Cargo.toml`/`package.json`,
//! guess a command, `bash` it, read the failure, retry the command. This tool does
//! the detection deterministically from marker files, runs the right command, and
//! returns a compact pass/fail summary — one tool call instead of four or five.
//!
//! Detection is by marker file, most-specific ecosystem first. The caller may pin
//! an `ecosystem` to skip detection, pass a `filter` to narrow the run, or raise
//! `timeout_secs` for a slow suite. Pass/fail is taken from the process exit code
//! (the ground truth); a framework-specific summary line is surfaced when found.

use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use pirs_agent::tool::{AgentTool, ExecutionMode, ToolExecContext, ToolOutput};

const DEFAULT_TIMEOUT_SECS: u64 = 300;

/// A detected test ecosystem: its identifier and the command to run.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Runner {
    ecosystem: &'static str,
    command: String,
}

pub struct RunTestsTool {
    cwd: PathBuf,
}

impl RunTestsTool {
    pub fn new(cwd: PathBuf) -> Self {
        RunTestsTool { cwd }
    }
}

/// Build the test command for a known ecosystem, folding in an optional filter.
/// Returns `None` if the ecosystem's marker isn't present in `root`.
fn runner_for(ecosystem: &str, root: &Path, filter: Option<&str>) -> Option<Runner> {
    let has = |f: &str| root.join(f).exists();
    let command = match ecosystem {
        "rust" if has("Cargo.toml") => match filter {
            // `cargo test <name>` filters by substring across the workspace.
            Some(f) => format!("cargo test {}", shell_quote(f)),
            None => "cargo test".to_string(),
        },
        "go" if has("go.mod") => match filter {
            Some(f) => format!("go test -run {} ./...", shell_quote(f)),
            None => "go test ./...".to_string(),
        },
        "node" if has("package.json") => {
            let base = if has("pnpm-lock.yaml") {
                "pnpm test"
            } else if has("yarn.lock") {
                "yarn test"
            } else {
                "npm test"
            };
            match filter {
                // Everything after `--` is forwarded to the underlying runner.
                Some(f) => format!("{base} -- {}", shell_quote(f)),
                None => base.to_string(),
            }
        }
        "python"
            if has("pyproject.toml")
                || has("setup.py")
                || has("setup.cfg")
                || has("tox.ini")
                || has("pytest.ini")
                || root.join("tests").is_dir() =>
        {
            match filter {
                Some(f) => format!("pytest -q -k {}", shell_quote(f)),
                None => "pytest -q".to_string(),
            }
        }
        // Lowest-priority fallback: a Makefile that wraps the real runner.
        "make" if has("Makefile") || has("makefile") => "make test".to_string(),
        _ => return None,
    };
    Some(Runner {
        ecosystem: match ecosystem {
            "rust" => "rust",
            "go" => "go",
            "node" => "node",
            "python" => "python",
            _ => "make",
        },
        command,
    })
}

/// Detection priority: language ecosystems before the generic Makefile fallback.
const ECOSYSTEMS: &[&str] = &["rust", "go", "node", "python", "make"];

/// Detect the project's test runner, or return the list of markers looked for.
fn detect(root: &Path, filter: Option<&str>) -> Result<Runner, String> {
    for eco in ECOSYSTEMS {
        if let Some(r) = runner_for(eco, root, filter) {
            return Ok(r);
        }
    }
    Err(
        "no test ecosystem detected (looked for Cargo.toml, go.mod, \
         package.json, pyproject.toml/setup.py/tests/, Makefile)"
            .to_string(),
    )
}

/// Public detect for strategy auto-verify (`--weak` without explicit `--verify`).
/// Returns `(ecosystem, command)` or `None` when no marker files are present.
pub fn detect_verify_command(root: &Path) -> Option<(String, String)> {
    detect(root, None)
        .ok()
        .map(|r| (r.ecosystem.to_string(), r.command))
}

/// Minimal POSIX single-quote escaping so a user filter can't break out of the
/// command. Wraps in single quotes and escapes embedded single quotes.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Pull a human-readable pass/fail summary line out of test output. Best-effort:
/// the exit code remains the source of truth for pass/fail.
fn summarize(ecosystem: &str, output: &str) -> Option<String> {
    let find = |needle: &str| {
        output
            .lines()
            .rev()
            .find(|l| l.contains(needle))
            .map(|l| l.trim().to_string())
    };
    match ecosystem {
        // cargo/nextest: "test result: ok. 12 passed; 0 failed; ..."
        "rust" => find("test result:"),
        // pytest summary: "===== 3 passed, 1 failed in 0.12s ====="
        "python" => find(" passed")
            .or_else(|| find(" failed"))
            .or_else(|| find(" error")),
        // go prints per-package ok/FAIL; surface the last verdict line.
        "go" => find("FAIL")
            .or_else(|| find("ok  "))
            .or_else(|| find("PASS")),
        _ => None,
    }
}

#[async_trait]
impl AgentTool for RunTestsTool {
    fn name(&self) -> &str {
        "run_tests"
    }

    fn description(&self) -> &str {
        "Detect this project's test framework from its marker files and run the \
         tests in one step. Optionally narrow with `filter` (a test name/substring) \
         or pin `ecosystem` (rust|go|node|python|make). Returns pass/fail plus a \
         summary — use this instead of manually inspecting the project and shelling \
         out a test command."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "filter": {
                    "type": "string",
                    "description": "Optional test name or substring to narrow the run."
                },
                "ecosystem": {
                    "type": "string",
                    "enum": ["rust", "go", "node", "python", "make"],
                    "description": "Pin the ecosystem instead of auto-detecting."
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Max seconds before the run is killed (default 300)."
                }
            },
            "additionalProperties": false
        })
    }

    fn execution_mode(&self) -> ExecutionMode {
        // A test run can write build artifacts; keep it off the parallel path.
        ExecutionMode::Sequential
    }

    async fn execute(&self, ctx: ToolExecContext) -> anyhow::Result<ToolOutput> {
        let filter = ctx.args.get("filter").and_then(|v| v.as_str());
        let timeout_secs = ctx
            .args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT_SECS);

        let runner = match ctx.args.get("ecosystem").and_then(|v| v.as_str()) {
            Some(eco) => runner_for(eco, &self.cwd, filter).ok_or_else(|| {
                anyhow::anyhow!("ecosystem {eco:?} pinned, but its marker file is not present")
            })?,
            None => detect(&self.cwd, filter).map_err(|e| anyhow::anyhow!(e))?,
        };

        ctx.emit_update(format!("running: {}", runner.command));
        let out = crate::bash::exec_local(
            &runner.command,
            &self.cwd,
            Some(Duration::from_secs(timeout_secs)),
        )
        .await?;

        let combined = format!("{}{}", out.stdout, out.stderr);
        let passed = matches!(out.code, Some(0)) && !out.timed_out;
        let summary = summarize(runner.ecosystem, &combined);

        let verdict = if out.timed_out {
            format!("TIMEOUT after {timeout_secs}s")
        } else if passed {
            "PASS".to_string()
        } else {
            match out.code {
                Some(n) => format!("FAIL (exit {n})"),
                None => "FAIL (killed by signal)".to_string(),
            }
        };

        let head = format!(
            "[{}] {} — {}{}",
            runner.ecosystem,
            runner.command,
            verdict,
            summary.map(|s| format!("\n{s}")).unwrap_or_default()
        );
        // Tail the raw output so a failure is debuggable without flooding context.
        let tail = tail_lines(&combined, 40);
        let text = if tail.is_empty() {
            head
        } else {
            format!("{head}\n\n{tail}")
        };

        Ok(ToolOutput::text(text).with_details(json!({
            "ecosystem": runner.ecosystem,
            "command": runner.command,
            "passed": passed,
            "exit_code": out.code,
            "timed_out": out.timed_out,
        })))
    }
}

/// Keep the last `n` lines of `s` — enough to see the failure without the whole log.
fn tail_lines(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn detects_rust_by_cargo_toml() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname='x'\n").unwrap();
        let r = detect(dir.path(), None).unwrap();
        assert_eq!(r.ecosystem, "rust");
        assert_eq!(r.command, "cargo test");
    }

    #[test]
    fn go_filter_uses_run_flag() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module x\n").unwrap();
        let r = detect(dir.path(), Some("TestFoo")).unwrap();
        assert_eq!(r.command, "go test -run 'TestFoo' ./...");
    }

    #[test]
    fn node_picks_the_lockfile_specific_runner() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();
        std::fs::write(dir.path().join("pnpm-lock.yaml"), "").unwrap();
        let r = detect(dir.path(), None).unwrap();
        assert_eq!(r.command, "pnpm test");
    }

    #[test]
    fn python_detected_by_a_tests_dir_alone() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join("tests")).unwrap();
        let r = detect(dir.path(), Some("edge")).unwrap();
        assert_eq!(r.ecosystem, "python");
        assert_eq!(r.command, "pytest -q -k 'edge'");
    }

    #[test]
    fn language_marker_beats_makefile_fallback() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname='x'\n").unwrap();
        std::fs::write(dir.path().join("Makefile"), "test:\n\techo hi\n").unwrap();
        // Rust wins over the generic make fallback.
        assert_eq!(detect(dir.path(), None).unwrap().ecosystem, "rust");
    }

    #[test]
    fn makefile_is_the_last_resort() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("Makefile"), "test:\n\techo hi\n").unwrap();
        let r = detect(dir.path(), None).unwrap();
        assert_eq!(r.ecosystem, "make");
        assert_eq!(r.command, "make test");
    }

    #[test]
    fn no_markers_reports_what_it_looked_for() {
        let dir = tempdir().unwrap();
        let err = detect(dir.path(), None).unwrap_err();
        assert!(err.contains("Cargo.toml"), "{err}");
        assert!(err.contains("go.mod"), "{err}");
    }

    #[test]
    fn detect_verify_command_public_api_for_weak_auto_verify() {
        let dir = tempdir().unwrap();
        assert!(detect_verify_command(dir.path()).is_none());
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname='x'\n").unwrap();
        let (eco, cmd) = detect_verify_command(dir.path()).expect("rust");
        assert_eq!(eco, "rust");
        assert_eq!(cmd, "cargo test");
    }

    #[test]
    fn filter_is_shell_quoted_against_injection() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "x").unwrap();
        let r = detect(dir.path(), Some("foo; rm -rf /")).unwrap();
        // The whole filter is a single quoted argument — no bare `;`.
        assert_eq!(r.command, "cargo test 'foo; rm -rf /'");
    }

    #[test]
    fn rust_summary_line_is_extracted() {
        let out = "running 3 tests\ntest result: ok. 3 passed; 0 failed; 0 ignored";
        assert_eq!(
            summarize("rust", out).unwrap(),
            "test result: ok. 3 passed; 0 failed; 0 ignored"
        );
    }

    #[test]
    fn tail_keeps_only_the_last_lines() {
        let s: String = (0..100).map(|i| format!("line{i}\n")).collect();
        let t = tail_lines(&s, 5);
        assert!(t.starts_with("line95"));
        assert!(t.ends_with("line99"));
    }

    fn make_available() -> bool {
        std::process::Command::new("make")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn ctx_for(args: Value) -> ToolExecContext {
        ToolExecContext {
            tool_call_id: "t".into(),
            args,
            cancel: tokio_util::sync::CancellationToken::new(),
            on_update: None,
        }
    }

    #[tokio::test]
    async fn executes_a_passing_make_target_end_to_end() {
        if !make_available() {
            eprintln!("skipping: `make` not installed");
            return;
        }
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("Makefile"), "test:\n\t@echo all good\n").unwrap();
        let tool = RunTestsTool::new(dir.path().to_path_buf());
        let out = tool.execute(ctx_for(json!({}))).await.unwrap();
        let details = out.details.unwrap();
        assert_eq!(details["passed"], json!(true));
        assert_eq!(details["ecosystem"], json!("make"));
    }

    #[tokio::test]
    async fn executes_a_failing_make_target_and_reports_fail() {
        if !make_available() {
            eprintln!("skipping: `make` not installed");
            return;
        }
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("Makefile"), "test:\n\t@echo boom; exit 1\n").unwrap();
        let tool = RunTestsTool::new(dir.path().to_path_buf());
        // A non-zero exit surfaces as an Err from exec_local's finish path? No —
        // exec_local returns the ExecOutput regardless of code, so we get Ok with
        // passed=false.
        let out = tool.execute(ctx_for(json!({}))).await.unwrap();
        let details = out.details.unwrap();
        assert_eq!(details["passed"], json!(false));
        // make exits non-zero on a failed recipe (2, wrapping the recipe's 1).
        assert_ne!(details["exit_code"], json!(0));
        assert!(details["exit_code"].as_i64().unwrap() > 0);
    }

    #[tokio::test]
    async fn pinned_ecosystem_without_its_marker_errors() {
        let dir = tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let tool = RunTestsTool::new(path.clone());
        let err = tool
            .execute(ctx_for(json!({ "ecosystem": "rust" })))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("marker file is not present"), "{err}");
    }
}
