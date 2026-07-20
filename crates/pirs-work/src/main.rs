//! pirs-work — Claude/Codex/Qoder/Kimi-class coding agent over pirs.

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use pirs_ai::OpenAiCompat;
use pirs_work::{apply_work_defaults, build_work_agent, resolve_work_strategy, WorkOptions};

#[derive(Parser, Debug)]
#[command(
    name = "pirs-work",
    about = "Coding work agent (repo tools + plan-exec + multi-model). Thin wrapper over pirs."
)]
struct Cli {
    /// Working directory (repo root). Default: cwd.
    #[arg(long, short = 'C')]
    cwd: Option<PathBuf>,

    /// Executor model (alias or id). Default: qwen3.5-plus
    #[arg(long, default_value = pirs_work::DEFAULT_MODEL)]
    model: String,

    /// Planner model for strategy RO phases. Default: deepseek-v4-pro
    #[arg(long, default_value = pirs_work::DEFAULT_PLAN_MODEL)]
    plan_model: String,

    /// Strategy name (default plan-exec).
    #[arg(long, default_value = pirs_work::DEFAULT_STRATEGY)]
    strategy: String,

    /// Max agent turns for the run.
    #[arg(long)]
    max_turns: Option<usize>,

    /// One tool call at a time (weaker models).
    #[arg(long)]
    sequential: bool,

    /// One-shot prompt. If omitted, prints usage and exits 2 (use pirs --mode tui for interactive).
    prompt: Vec<String>,
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
    let cwd = cli
        .cwd
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // Load secrets + registry the same way pirs does (best-effort via env).
    // Work binary expects keys in the environment or ~/.pirs/secrets.env —
    // users typically `source ~/.pirs/secrets.env` or rely on shell config.
    load_secrets_env_best_effort();

    let opts = apply_work_defaults(WorkOptions {
        cwd: cwd.clone(),
        model: cli.model.clone(),
        plan_model: if cli.plan_model.is_empty() {
            None
        } else {
            Some(cli.plan_model.clone())
        },
        strategy: cli.strategy.clone(),
        prompt: {
            let p = cli.prompt.join(" ");
            if p.is_empty() {
                None
            } else {
                Some(p)
            }
        },
        max_turns: cli.max_turns.or(Some(40)),
        sequential: cli.sequential,
    });

    let strategy = resolve_work_strategy(&opts)?;
    eprintln!(
        "[pirs-work: cwd={} model={} plan_model={:?} strategy={} phases={}]",
        opts.cwd.display(),
        opts.model,
        opts.plan_model,
        strategy.name,
        strategy.steps.len()
    );

    let Some(prompt) = opts.prompt.clone() else {
        eprintln!(
            "usage: pirs-work [OPTIONS] <prompt…>\n\
             interactive coding UI: pirs --mode tui --strategy plan-exec --model … --plan-model …\n\
             defaults: model={} plan_model={} strategy={}",
            pirs_work::DEFAULT_MODEL,
            pirs_work::DEFAULT_PLAN_MODEL,
            pirs_work::DEFAULT_STRATEGY
        );
        std::process::exit(2);
    };

    // Prefer registry routing via pirs main for full multi-backend; here we use
    // OpenAI-compatible default (OPENAI_BASE_URL / keys from env). For aliases,
    // users should set OPENAI_BASE_URL to the right gateway or use `pirs` CLI.
    let base = std::env::var("OPENAI_BASE_URL").ok();
    let provider = Arc::new(OpenAiCompat::new(base).with_max_retries(2));
    let mut agent = build_work_agent(provider, &opts);

    // Inject strategy plan pin into system prompt note (full phase runner is on `pirs`).
    // For true multi-phase plan-exec with plan-model, shell out to pirs when available.
    if let Ok(pirs_bin) = find_pirs_bin() {
        let status = std::process::Command::new(&pirs_bin)
            .current_dir(&opts.cwd)
            .arg("--strategy")
            .arg(&opts.strategy)
            .arg("--model")
            .arg(&opts.model)
            .arg("--plan-model")
            .arg(opts.plan_model.as_deref().unwrap_or(""))
            .arg("--max-turns")
            .arg(opts.max_turns.unwrap_or(40).to_string())
            .arg(&prompt)
            .status();
        match status {
            Ok(s) if s.success() => return Ok(()),
            Ok(s) => std::process::exit(s.code().unwrap_or(1)),
            Err(e) => {
                eprintln!("[pirs-work: pirs bin failed ({e}); falling back to monolithic agent]");
            }
        }
    }

    let msgs = agent.prompt(&prompt).await?;
    // Print last assistant text.
    for m in msgs.iter().rev() {
        if let pirs_ai::Message::Assistant(a) = m {
            let t = a.text();
            if !t.trim().is_empty() {
                println!("{t}");
                break;
            }
        }
    }
    Ok(())
}

fn find_pirs_bin() -> Result<PathBuf, ()> {
    if let Ok(p) = std::env::var("PIRS_BIN") {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Ok(pb);
        }
    }
    // Sibling of this binary (cargo target dir).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let cand = dir.join("pirs");
            if cand.is_file() {
                return Ok(cand);
            }
        }
    }
    which("pirs").ok_or(())
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
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
