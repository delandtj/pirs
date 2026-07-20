//! pirs-work — coding agent defaults and composition (no bloat).
//!
//! Product class: Claude Code / Codex / Qoder / Kimi Code style **repo work**.
//! This crate only sets sensible defaults and wires existing pirs pieces.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pirs_agent::strategy::{pin_plan_model, Strategy, ToolScope};
use pirs_agent::{Agent, AgentTool};
use pirs_ai::LlmProvider;

/// Default strategy for coding work (plan then execute).
pub const DEFAULT_STRATEGY: &str = "plan-exec";

/// Default exec model alias (cheap).
pub const DEFAULT_MODEL: &str = "qwen3.5-plus";

/// Default planner model alias (strong).
pub const DEFAULT_PLAN_MODEL: &str = "deepseek-v4-pro";

/// CLI / library options for a work session.
#[derive(Debug, Clone)]
pub struct WorkOptions {
    pub cwd: PathBuf,
    pub model: String,
    pub plan_model: Option<String>,
    pub strategy: String,
    /// One-shot prompt; None means interactive intent (caller runs REPL).
    pub prompt: Option<String>,
    pub max_turns: Option<usize>,
    pub sequential: bool,
}

impl Default for WorkOptions {
    fn default() -> Self {
        WorkOptions {
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            model: DEFAULT_MODEL.into(),
            plan_model: Some(DEFAULT_PLAN_MODEL.into()),
            strategy: DEFAULT_STRATEGY.into(),
            prompt: None,
            max_turns: Some(40),
            sequential: false,
        }
    }
}

/// Apply coding-work defaults without clobbering explicit non-empty fields.
pub fn apply_work_defaults(mut opts: WorkOptions) -> WorkOptions {
    if opts.model.is_empty() {
        opts.model = DEFAULT_MODEL.into();
    }
    if opts.strategy.is_empty() {
        opts.strategy = DEFAULT_STRATEGY.into();
    }
    if opts.plan_model.as_ref().is_some_and(|s| s.is_empty()) {
        opts.plan_model = Some(DEFAULT_PLAN_MODEL.into());
    }
    if opts.plan_model.is_none() {
        opts.plan_model = Some(DEFAULT_PLAN_MODEL.into());
    }
    opts
}

/// Core coding tools for a workspace (read/edit/shell/search).
pub fn coding_tools(cwd: &Path) -> Vec<Arc<dyn AgentTool>> {
    pirs_tools::default_tools(cwd.to_path_buf())
}

/// Build an [`Agent`] for coding work with the given provider and options.
pub fn build_work_agent(
    provider: Arc<dyn LlmProvider>,
    opts: &WorkOptions,
) -> Agent {
    let tools = coding_tools(&opts.cwd);
    let mut agent = Agent::new(provider, opts.model.clone())
        .with_system_prompt(coding_system_prompt(&opts.cwd))
        .with_tools(tools);
    if let Some(n) = opts.max_turns {
        agent.budgets.max_turns = Some(n);
    }
    if opts.sequential {
        agent = agent.with_tool_execution(pirs_agent::ExecutionMode::Sequential);
    }
    agent
}

/// Resolve built-in strategy and pin plan model onto read-only phases.
pub fn resolve_work_strategy(opts: &WorkOptions) -> anyhow::Result<Strategy> {
    let mut s = pirs_rhai::builtins::builtin(&opts.strategy)
        .or_else(|| pirs_rhai::discover::resolve_strategy(&opts.strategy, &opts.cwd).ok())
        .ok_or_else(|| anyhow::anyhow!("unknown strategy {:?}", opts.strategy))?;
    if let Some(pm) = &opts.plan_model {
        pin_plan_model(&mut s, pm);
    }
    Ok(s)
}

/// Count read-only vs full phases after plan-model pin (for tests / diagnostics).
pub fn phase_scope_summary(strategy: &Strategy) -> (usize, usize, Vec<Option<String>>) {
    let mut ro = 0usize;
    let mut full = 0usize;
    let mut models = Vec::new();
    for step in &strategy.steps {
        match step {
            pirs_agent::strategy::Step::Solo(p) => {
                match p.scope {
                    ToolScope::ReadOnly => ro += 1,
                    ToolScope::Full => full += 1,
                }
                models.push(p.model.clone());
            }
            pirs_agent::strategy::Step::Fan { branches, .. } => {
                for p in branches {
                    match p.scope {
                        ToolScope::ReadOnly => ro += 1,
                        ToolScope::Full => full += 1,
                    }
                    models.push(p.model.clone());
                }
            }
        }
    }
    (ro, full, models)
}

fn coding_system_prompt(cwd: &Path) -> String {
    format!(
        "You are pirs-work, a coding agent working in `{}`.\n\
         Prefer small, correct edits. Use tools to read and search before writing.\n\
         Fix source, not tests, unless the user asked to change tests.\n\
         Run project tests after edits when possible. Be concise.",
        cwd.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pirs_agent::strategy::ToolScope;

    #[test]
    fn defaults_are_coding_oriented() {
        let mut o = WorkOptions::default();
        o.model.clear();
        o.strategy.clear();
        o.plan_model = None;
        let o = apply_work_defaults(o);
        assert_eq!(o.model, DEFAULT_MODEL);
        assert_eq!(o.strategy, DEFAULT_STRATEGY);
        assert_eq!(o.plan_model.as_deref(), Some(DEFAULT_PLAN_MODEL));
    }

    #[test]
    fn coding_tools_include_core_names() {
        let dir = tempfile::tempdir().unwrap();
        let tools = coding_tools(dir.path());
        let names: Vec<_> = tools.iter().map(|t| t.name().to_string()).collect();
        for need in ["read", "write", "edit", "bash", "grep", "find", "ls"] {
            assert!(
                names.iter().any(|n| n == need),
                "missing tool {need} in {names:?}"
            );
        }
    }

    #[test]
    fn plan_exec_strategy_pins_plan_model_on_readonly_only() {
        let opts = WorkOptions {
            strategy: "plan-exec".into(),
            plan_model: Some("deepseek-v4-pro".into()),
            model: "qwen3.5-plus".into(),
            ..WorkOptions::default()
        };
        let s = resolve_work_strategy(&opts).expect("plan-exec builtin");
        assert_eq!(s.name, "plan-exec");
        let (ro, full, models) = phase_scope_summary(&s);
        assert!(ro >= 1, "need plan phase");
        assert!(full >= 1, "need exec phase");
        // First phase is plan (readonly) with pin; last full has no pin.
        match &s.steps[0] {
            pirs_agent::strategy::Step::Solo(p) => {
                assert_eq!(p.scope, ToolScope::ReadOnly);
                assert_eq!(p.model.as_deref(), Some("deepseek-v4-pro"));
            }
            _ => panic!("expected solo plan"),
        }
        match s.steps.last() {
            Some(pirs_agent::strategy::Step::Solo(p)) => {
                assert_eq!(p.scope, ToolScope::Full);
                assert!(p.model.is_none(), "exec keeps run default model");
            }
            _ => panic!("expected solo exec"),
        }
        assert!(models.iter().any(|m| m.as_deref() == Some("deepseek-v4-pro")));
    }

    /// Real agent + tools + mock provider: one-shot coding path without live API.
    #[tokio::test]
    async fn work_agent_one_shot_with_mock_provider() {
        use async_trait::async_trait;
        use pirs_ai::{
            AssistantMessage, CompletionOptions, ContentBlock, Context, LlmProvider, StopReason,
            StreamEvent,
        };
        use std::sync::Mutex;

        struct Mock {
            seen: Mutex<usize>,
        }
        #[async_trait]
        impl LlmProvider for Mock {
            async fn stream(
                &self,
                _model: &str,
                _ctx: &Context,
                _opts: &CompletionOptions,
                _cancel: tokio_util::sync::CancellationToken,
            ) -> futures::stream::BoxStream<'static, StreamEvent> {
                *self.seen.lock().unwrap() += 1;
                let msg = AssistantMessage {
                    content: vec![ContentBlock::text("hello from pirs-work")],
                    stop_reason: StopReason::Stop,
                    ..Default::default()
                };
                Box::pin(futures::stream::iter(vec![
                    StreamEvent::Start,
                    StreamEvent::TextDelta("hello from pirs-work".into()),
                    StreamEvent::Done(Box::new(msg)),
                ]))
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let opts = WorkOptions {
            cwd: dir.path().to_path_buf(),
            model: "mock".into(),
            plan_model: None,
            strategy: "plan-exec".into(),
            prompt: Some("hi".into()),
            max_turns: Some(3),
            sequential: true,
        };
        let provider = Arc::new(Mock {
            seen: Mutex::new(0),
        });
        let mut agent = build_work_agent(provider.clone(), &opts);
        let msgs = agent.prompt("say hi").await.unwrap();
        assert!(
            msgs.iter().any(|m| matches!(
                m,
                pirs_ai::Message::Assistant(a) if a.text().contains("pirs-work")
            )),
            "expected assistant reply: {msgs:?}"
        );
        assert!(*provider.seen.lock().unwrap() >= 1);
        assert!(!coding_tools(dir.path()).is_empty());
    }
}
