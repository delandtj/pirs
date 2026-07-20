//! pirs-claw — lean personal assistant CLI.

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use pirs_ai::OpenAiCompat;
use pirs_agent::Agent;
use pirs_claw::{
    claw_system_prompt, default_schedule_path, default_session_path, ScheduleStore, SessionStore,
};

#[derive(Parser)]
#[command(
    name = "pirs-claw",
    about = "Lean personal assistant (OpenClaw/Hermes alternative): chat + memory + schedules. No channel zoo."
)]
struct Cli {
    #[arg(long, global = true)]
    state_dir: Option<PathBuf>,

    #[arg(long, global = true, default_value = "qwen3.5-plus")]
    model: String,

    #[command(subcommand)]
    cmd: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// One-shot chat (appends to durable session).
    Chat {
        /// User message.
        message: Vec<String>,
    },
    /// Print session history.
    History {
        #[arg(long, default_value_t = 20)]
        last: usize,
    },
    /// Schedule management.
    Schedule {
        #[command(subcommand)]
        cmd: ScheduleCmd,
    },
}

#[derive(Subcommand)]
enum ScheduleCmd {
    /// Add a job. Fires after `in_secs`, optionally every `every` seconds.
    Add {
        prompt: Vec<String>,
        #[arg(long, default_value_t = 0)]
        in_secs: u64,
        #[arg(long, default_value_t = 0)]
        every: u64,
    },
    List,
    /// Fire all due jobs once (no daemon). Prints prompts that would run / runs them.
    Tick {
        /// Actually call the model for due jobs (default: dry-run print only).
        #[arg(long)]
        run: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn".into()),
        )
        .init();

    let cli = Cli::parse();
    load_secrets_env_best_effort();

    let state = cli
        .state_dir
        .clone()
        .unwrap_or_else(pirs_claw::default_state_dir);
    std::fs::create_dir_all(&state)?;
    let session_path = state.join("session.jsonl");
    let schedule_path = state.join("schedule.json");
    // Prefer explicit paths when using defaults for docs consistency.
    let _ = (default_session_path(), default_schedule_path());

    match cli.cmd {
        Commands::Chat { message } => {
            let text = message.join(" ");
            if text.is_empty() {
                anyhow::bail!("usage: pirs-claw chat <message>");
            }
            let store = SessionStore::open(&session_path)?;
            store.append("user", &text)?;

            let (base, key) = resolve_provider_and_key();
            let provider = Arc::new(OpenAiCompat::new(base).with_max_retries(2));
            let completion = pirs_ai::CompletionOptions {
                api_key: key,
                ..Default::default()
            };
            let mut agent = Agent::new(provider, &cli.model)
                .with_system_prompt(claw_system_prompt())
                .with_tools(pirs_tools::default_tools(std::env::current_dir()?))
                .with_completion(completion);
            // Hydrate prior session (skip last user we just appended — prompt adds it).
            let prior = store.load()?;
            if prior.len() > 1 {
                let mut msgs = store.to_agent_messages()?;
                if let Some(pirs_ai::Message::User(_)) = msgs.last() {
                    msgs.pop(); // current prompt
                }
                agent.messages = msgs;
            }

            let new_msgs = agent.prompt(&text).await?;
            let reply = new_msgs
                .iter()
                .rev()
                .find_map(|m| match m {
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
                .unwrap_or_else(|| "(no reply)".into());
            store.append("assistant", &reply)?;
            println!("{reply}");
            eprintln!("[pirs-claw: session {}]", store.path().display());
        }
        Commands::History { last } => {
            let store = SessionStore::open(&session_path)?;
            let lines = store.load()?;
            let start = lines.len().saturating_sub(last);
            for l in &lines[start..] {
                println!("[{}] {}: {}", l.ts, l.role, l.text);
            }
        }
        Commands::Schedule { cmd } => {
            let store = ScheduleStore::open(&schedule_path)?;
            match cmd {
                ScheduleCmd::Add {
                    prompt,
                    in_secs,
                    every,
                } => {
                    let p = prompt.join(" ");
                    let e = store.add(&p, every, in_secs)?;
                    println!(
                        "scheduled {} next_fire={} every_secs={}",
                        e.id, e.next_fire, e.every_secs
                    );
                }
                ScheduleCmd::List => {
                    for j in store.list()? {
                        println!(
                            "{} enabled={} next={} every={} | {}",
                            j.id, j.enabled, j.next_fire, j.every_secs, j.prompt
                        );
                    }
                }
                ScheduleCmd::Tick { run } => {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let due = store.due(now)?;
                    if due.is_empty() {
                        println!("no due jobs");
                    }
                    for j in due {
                        println!("due {}: {}", j.id, j.prompt);
                        if run {
                            // Re-enter chat path for the job prompt.
                            let status = std::process::Command::new(std::env::current_exe()?)
                                .arg("--model")
                                .arg(&cli.model)
                                .arg("--state-dir")
                                .arg(&state)
                                .arg("chat")
                                .arg(&j.prompt)
                                .status()?;
                            if !status.success() {
                                eprintln!("[tick] job {} failed", j.id);
                            }
                        }
                        store.mark_fired(&j.id, now)?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn resolve_provider_and_key() -> (Option<String>, Option<String>) {
    if let Ok(base) = std::env::var("OPENAI_BASE_URL") {
        let key = std::env::var("OPENAI_API_KEY")
            .ok()
            .or_else(|| std::env::var("DASHSCOPE_API_KEY").ok())
            .or_else(|| std::env::var("DEEPSEEK_API_KEY").ok());
        return (Some(base), key);
    }
    if std::env::var("DASHSCOPE_API_KEY").is_ok() {
        return (
            Some("https://coding-intl.dashscope.aliyuncs.com/v1".into()),
            std::env::var("DASHSCOPE_API_KEY").ok(),
        );
    }
    if std::env::var("DEEPSEEK_API_KEY").is_ok() {
        return (
            Some("https://api.deepseek.com/v1".into()),
            std::env::var("DEEPSEEK_API_KEY").ok(),
        );
    }
    (
        None,
        std::env::var("OPENAI_API_KEY").ok(),
    )
}

fn load_secrets_env_best_effort() {
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let path = PathBuf::from(home).join(".pirs").join("secrets.env");
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let body = line.strip_prefix("export ").unwrap_or(line);
        let Some((k, v)) = body.split_once('=') else {
            continue;
        };
        let k = k.trim();
        if std::env::var_os(k).is_some() {
            continue;
        }
        let mut v = v.trim().to_string();
        if (v.starts_with('\'') && v.ends_with('\'')) || (v.starts_with('"') && v.ends_with('"')) {
            v = v[1..v.len() - 1].to_string();
        }
        if v.starts_with("${") && v.ends_with('}') {
            let refn = &v[2..v.len() - 1];
            v = std::env::var(refn).unwrap_or_default();
            if v.is_empty() {
                continue;
            }
        }
        std::env::set_var(k, v);
    }
}
