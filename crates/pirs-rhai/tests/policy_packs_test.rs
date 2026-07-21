//! Policy packs: strict-plan (add-only denials), session-discipline (steering).

use std::sync::Arc;

use pirs_rhai::ExtensionHost;
use serde_json::json;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn load(name: &str) -> Arc<ExtensionHost> {
    pirs_rhai::register_core_host_apis();
    let path = format!("{}/../../extensions/{name}", env!("CARGO_MANIFEST_DIR"));
    let mut host = ExtensionHost::new();
    host.load_source(&std::fs::read_to_string(&path).unwrap(), path)
        .unwrap();
    Arc::new(host)
}

#[test]
fn strict_plan_blocks_web_search_when_profile_plan() {
    let _g = ENV_LOCK.lock().unwrap();
    std::env::set_var("PIRS_AGENT_PROFILE", "plan");
    let host = load("strict-plan.rhai");
    let before = host.hooks().before_tool_call.expect("hook");
    let deny = before("1", "web_search", &json!({"query": "x"}));
    assert!(
        deny.as_ref().is_some_and(|s| s.contains("strict-plan")),
        "got {deny:?}"
    );
    // read is not in EXTRA_PLAN_BLOCK — pack must not invent denials for it.
    assert!(before("1", "read", &json!({"path": "a"})).is_none());
    std::env::remove_var("PIRS_AGENT_PROFILE");
}

#[test]
fn strict_plan_idle_when_profile_default() {
    let _g = ENV_LOCK.lock().unwrap();
    std::env::set_var("PIRS_AGENT_PROFILE", "default");
    std::env::remove_var("PIRS_STRICT_PLAN");
    let host = load("strict-plan.rhai");
    let before = host.hooks().before_tool_call.expect("hook");
    assert!(before("1", "web_search", &json!({"query": "x"})).is_none());
    std::env::remove_var("PIRS_AGENT_PROFILE");
}

#[test]
fn session_discipline_steers_todo_after_mutates() {
    let _g = ENV_LOCK.lock().unwrap();
    let host = load("session-discipline.rhai");
    let hooks = host.hooks();
    let before = hooks.before_tool_call.expect("before");
    let steer = hooks.get_steering_messages.expect("steering");
    // Two mutates without todo.
    before("1", "edit", &json!({"path": "a"}));
    before("2", "write", &json!({"path": "b"}));
    let msgs = steer();
    assert!(
        msgs.iter().any(|m| {
            let t = match m {
                pirs_ai::Message::User(u) => match &u.content {
                    pirs_ai::UserContent::Text(s) => s.clone(),
                    _ => String::new(),
                },
                _ => String::new(),
            };
            t.contains("todo")
        }),
        "expected todo steering, got {msgs:?}"
    );
}

#[test]
fn browser_cdp_workflow_steers_on_first_call() {
    let _g = ENV_LOCK.lock().unwrap();
    let host = load("browser-cdp-workflow.rhai");
    let hooks = host.hooks();
    let before = hooks.before_tool_call.expect("before");
    let steer = hooks.get_steering_messages.expect("steering");
    before("1", "browser_cdp", &json!({"action": "connect"}));
    let msgs = steer();
    assert!(
        msgs.iter().any(|m| {
            let t = match m {
                pirs_ai::Message::User(u) => match &u.content {
                    pirs_ai::UserContent::Text(s) => s.clone(),
                    _ => String::new(),
                },
                _ => String::new(),
            };
            t.contains("browser_cdp") || t.contains("PIRS_BROWSER_CDP")
        }),
        "expected cdp recipe steer, got {msgs:?}"
    );
}
