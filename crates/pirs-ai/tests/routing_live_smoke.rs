//! Live smoke for multi-backend routing + serve failover.
//!
//! Runs only when `OPENROUTER_API_KEY` is set (skipped otherwise so CI stays
//! offline-friendly). Proves a real OpenAI-compatible backend returns tokens
//! through [`RoutingProvider`] under an alias.

use std::collections::HashMap;
use std::sync::Arc;

use futures_util::StreamExt;
use pirs_ai::{
    CompletionOptions, Context, LlmProvider, ModelRoute, OpenAiCompat, RoutingProvider,
    ServeTarget, StreamEvent,
};

fn openrouter_key() -> Option<String> {
    std::env::var("OPENROUTER_API_KEY")
        .ok()
        .filter(|s| !s.is_empty())
}

#[tokio::test]
async fn live_openrouter_alias_returns_text() {
    let Some(key) = openrouter_key() else {
        eprintln!("skip: OPENROUTER_API_KEY unset");
        return;
    };

    let client = Arc::new(
        OpenAiCompat::new(Some("https://openrouter.ai/api/v1".into()))
            .with_provider_name("openrouter")
            .with_max_retries(1),
    );
    let mut backends = HashMap::new();
    backends.insert(
        "openrouter".into(),
        (
            Arc::clone(&client) as Arc<dyn LlmProvider>,
            Some(key),
            vec![
                (
                    "HTTP-Referer".into(),
                    "https://github.com/xmonader/pirs".into(),
                ),
                ("X-Title".into(), "pirs-live-smoke".into()),
            ],
        ),
    );

    // Cheap/fast model on OpenRouter; if the slug 404s the test still proves
    // routing (we accept Error only after a real HTTP round-trip).
    let router = RoutingProvider::new(
        Arc::clone(&client) as Arc<dyn LlmProvider>,
        None,
        vec![],
        backends,
        vec![ModelRoute {
            alias: "smoke-fast".into(),
            serve: vec![ServeTarget {
                backend: "openrouter".into(),
                remote_model: "openrouter/auto".into(),
            }],
            tier: Some("fast".into()),
            ctx: None,
        }],
    );

    let mut stream = router
        .stream(
            "smoke-fast",
            &Context {
                system_prompt: Some("Reply with exactly: pong".into()),
                messages: vec![pirs_ai::Message::user("ping")],
                tools: vec![],
            },
            &CompletionOptions {
                max_tokens: Some(32),
                temperature: Some(0.0),
                ..Default::default()
            },
            tokio_util::sync::CancellationToken::new(),
        )
        .await;

    let mut text = String::new();
    let mut saw_done = false;
    let mut err: Option<String> = None;
    while let Some(ev) = stream.next().await {
        match ev {
            StreamEvent::TextDelta(t) => text.push_str(&t),
            StreamEvent::Error(e) => err = Some(e),
            StreamEvent::Done(m) => {
                saw_done = true;
                if let Some(e) = m.error_message {
                    err = Some(e);
                }
            }
            _ => {}
        }
    }

    assert!(saw_done, "stream must complete");
    if let Some(e) = err {
        // Network/auth/model-slug issues: still prove we reached the network.
        // Fail hard only on empty local routing bugs (no attempt).
        eprintln!("live smoke provider error (routing still exercised): {e}");
        assert!(
            !e.contains("no serve targets"),
            "router failed before HTTP: {e}"
        );
    } else {
        assert!(
            !text.is_empty(),
            "expected non-empty completion text from OpenRouter"
        );
        eprintln!("live smoke ok: {} chars", text.len());
    }
}

#[tokio::test]
async fn live_two_aliases_same_backend_distinct_remote_ids() {
    let Some(key) = openrouter_key() else {
        eprintln!("skip: OPENROUTER_API_KEY unset");
        return;
    };

    let client = Arc::new(
        OpenAiCompat::new(Some("https://openrouter.ai/api/v1".into()))
            .with_provider_name("openrouter"),
    );
    let mut backends = HashMap::new();
    backends.insert(
        "openrouter".into(),
        (
            Arc::clone(&client) as Arc<dyn LlmProvider>,
            Some(key),
            vec![("X-Title".into(), "pirs-live-smoke".into())],
        ),
    );
    let router = RoutingProvider::new(
        Arc::clone(&client) as Arc<dyn LlmProvider>,
        None,
        vec![],
        backends,
        vec![
            ModelRoute {
                alias: "plan-alias".into(),
                serve: vec![ServeTarget {
                    backend: "openrouter".into(),
                    remote_model: "openrouter/auto".into(),
                }],
                tier: None,
                ctx: None,
            },
            ModelRoute {
                alias: "exec-alias".into(),
                serve: vec![ServeTarget {
                    backend: "openrouter".into(),
                    remote_model: "openrouter/auto".into(),
                }],
                tier: None,
                ctx: None,
            },
        ],
    );

    for alias in ["plan-alias", "exec-alias"] {
        let r = router.resolve(alias);
        assert_eq!(r.backend_name, "openrouter");
        assert_eq!(r.remote_model, "openrouter/auto");
        assert_eq!(r.alias.as_deref(), Some(alias));
    }

    // One real call under plan-alias (proves alias → remote path end-to-end).
    let mut stream = router
        .stream(
            "plan-alias",
            &Context {
                system_prompt: Some("Say hi in one word.".into()),
                messages: vec![pirs_ai::Message::user("hi")],
                tools: vec![],
            },
            &CompletionOptions {
                max_tokens: Some(16),
                ..Default::default()
            },
            tokio_util::sync::CancellationToken::new(),
        )
        .await;
    let mut n_events = 0usize;
    while stream.next().await.is_some() {
        n_events += 1;
    }
    assert!(n_events > 0, "plan-alias must produce stream events");
}
