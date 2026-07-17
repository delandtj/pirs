use std::path::PathBuf;

use async_trait::async_trait;
use pirs_agent::{AgentTool, ToolExecContext, ToolOutput};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use pirs_agent::jobs::{self, JobStatus};

#[derive(Deserialize, JsonSchema)]
struct NoArgs {}

#[derive(Deserialize, JsonSchema)]
struct JobOutputArgs {
    /// Job id
    id: u64,
    /// Max lines from the end
    limit: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
struct JobKillArgs {
    /// Job id
    id: u64,
}

#[derive(Deserialize, JsonSchema)]
struct JobSteerArgs {
    /// Job id
    id: u64,
    /// Message to steer the background agent with
    message: String,
}

pub struct JobsTool;
pub struct JobOutputTool;
pub struct JobKillTool;
pub struct JobSteerTool;

#[async_trait]
impl AgentTool for JobsTool {
    fn name(&self) -> &str {
        "jobs"
    }
    fn description(&self) -> &str {
        "List background jobs (bash jobs and background sub-agents) with their status."
    }
    fn parameters(&self) -> Value {
        serde_json::to_value(schemars::schema_for!(NoArgs)).unwrap()
    }
    fn prompt_snippet(&self) -> Option<&str> {
        Some("jobs: list background jobs")
    }
    async fn execute(&self, _ctx: ToolExecContext) -> anyhow::Result<ToolOutput> {
        let lines = jobs::registry().list();
        if lines.is_empty() {
            return Ok(ToolOutput::text("no background jobs"));
        }
        Ok(ToolOutput::text(lines.join("\n")))
    }
}

#[async_trait]
impl AgentTool for JobOutputTool {
    fn name(&self) -> &str {
        "job_output"
    }
    fn description(&self) -> &str {
        "Read the current output of a background job (bash output, or a background agent's progress)."
    }
    fn parameters(&self) -> Value {
        serde_json::to_value(schemars::schema_for!(JobOutputArgs)).unwrap()
    }
    fn prompt_snippet(&self) -> Option<&str> {
        Some("job_output: read a background job's output")
    }
    async fn execute(&self, ctx: ToolExecContext) -> anyhow::Result<ToolOutput> {
        let args: JobOutputArgs = serde_json::from_value(ctx.args)?;
        let Some(job) = jobs::registry().get(args.id) else {
            anyhow::bail!("no such job: {}", args.id);
        };
        let (status_line, output_path, progress) = {
            let j = job.lock().unwrap();
            (j.status_line(), j.output_path.clone(), j.progress.clone())
        };
        if let Some(progress) = progress {
            let text = progress.lock().unwrap().clone();
            return Ok(ToolOutput::text(format!("{status_line}\n\n{text}")));
        }
        let content = std::fs::read_to_string(&output_path)
            .unwrap_or_else(|_| "(no output yet)".to_string());
        let limit = args.limit.unwrap_or(50);
        let lines: Vec<&str> = content.lines().collect();
        let start = lines.len().saturating_sub(limit);
        let tail = lines[start..].join("\n");
        Ok(ToolOutput::text(format!("{status_line}\n\n{tail}")))
    }
}

#[async_trait]
impl AgentTool for JobKillTool {
    fn name(&self) -> &str {
        "job_kill"
    }
    fn description(&self) -> &str {
        "Kill a background job by id."
    }
    fn parameters(&self) -> Value {
        serde_json::to_value(schemars::schema_for!(JobKillArgs)).unwrap()
    }
    async fn execute(&self, ctx: ToolExecContext) -> anyhow::Result<ToolOutput> {
        let args: JobKillArgs = serde_json::from_value(ctx.args)?;
        let Some(job) = jobs::registry().get(args.id) else {
            anyhow::bail!("no such job: {}", args.id);
        };
        let (pid, kind) = {
            let j = job.lock().unwrap();
            (j.pid, j.kind)
        };
        match kind {
            jobs::JobKind::Bash => {
                if let Some(pid) = pid {
                    #[cfg(unix)]
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGKILL);
                    }
                }
            }
            jobs::JobKind::Agent => {
                let _ = jobs::registry().steer(args.id, "/cancel");
            }
        }
        jobs::registry().set_status(args.id, JobStatus::Killed);
        Ok(ToolOutput::text(format!("job {} killed", args.id)))
    }
}

#[async_trait]
impl AgentTool for JobSteerTool {
    fn name(&self) -> &str {
        "job_steer"
    }
    fn description(&self) -> &str {
        "Send a steering message to a running background sub-agent (only agent jobs are steerable)."
    }
    fn parameters(&self) -> Value {
        serde_json::to_value(schemars::schema_for!(JobSteerArgs)).unwrap()
    }
    fn prompt_snippet(&self) -> Option<&str> {
        Some("job_steer: send a message to a running background agent")
    }
    async fn execute(&self, ctx: ToolExecContext) -> anyhow::Result<ToolOutput> {
        let args: JobSteerArgs = serde_json::from_value(ctx.args)?;
        jobs::registry()
            .steer(args.id, &args.message)
            .map_err(anyhow::Error::msg)?;
        Ok(ToolOutput::text(format!("steered job {}", args.id)))
    }
}

pub fn bash_job_output_path(id: u64) -> PathBuf {
    std::env::temp_dir().join(format!("pirs-job-{id}.log"))
}

pub fn spawn_bash_job(cwd: &std::path::Path, command: &str) -> anyhow::Result<(u64, PathBuf)> {
    let shell = std::env::var("PIRS_SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    let (id, job) = jobs::registry().register(
        jobs::JobKind::Bash,
        command.chars().take(80).collect(),
        PathBuf::new(),
        None,
    );
    let path = bash_job_output_path(id);
    let out_file = std::fs::File::create(&path)?;
    let err_file = out_file.try_clone()?;

    let mut cmd = std::process::Command::new(&shell);
    cmd.arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(out_file))
        .stderr(std::process::Stdio::from(err_file));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd.spawn()?;
    {
        let mut j = job.lock().unwrap();
        j.pid = Some(child.id());
        j.output_path = path.clone();
    }
    let command_owned = command.to_string();
    std::thread::spawn(move || {
        let code = child
            .wait()
            .ok()
            .and_then(|s| s.code())
            .unwrap_or(-1);
        jobs::registry().set_status(id, JobStatus::Exited(code));
        jobs::registry()
            .notify(format!("background job #{id} exited (code {code}): {command_owned}"));
    });
    Ok((id, path))
}

pub fn tools() -> Vec<Box<dyn AgentTool>> {
    vec![
        Box::new(JobsTool),
        Box::new(JobOutputTool),
        Box::new(JobKillTool),
        Box::new(JobSteerTool),
    ]
}
