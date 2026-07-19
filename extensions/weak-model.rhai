// weak-model.rhai — loop hardening for weaker models.
// Install: cp to .pirs/extensions/ (project) or ~/.pirs/extensions/ (global).
// Three behaviors: loop detection, verify-after-edit, plan pinning.

const PLAN_PIN = "[current-plan pinned by extension]";
const MAX_REPEAT = 3;

// --- 1) Loop detection: block the same tool call repeated MAX_REPEAT times --
fn on_tool_call(id, name, args) {
    let key = name + "|" + args.to_json();
    let counts = if state_has("call_counts") { state_get("call_counts") } else { #{} };
    let n = (counts[key] ?? 0) + 1;
    counts[key] = n;
    state_set("call_counts", counts);
    if n >= MAX_REPEAT {
        state_set("nudge", true);
        return #{
            block: true,
            reason: `This exact call was already made ${n} times and keeps failing or changing nothing. Stop: inspect the last error, change strategy, or ask the user.`
        };
    }
    ()
}

// --- 2) Verify-after-edit: force a build/test run after file mutations ------
fn on_tool_result(id, name, result) {
    if (name == "edit" || name == "write") && !result.isError {
        state_set("needs_verify", true);
    }
    ()
}

fn on_steering() {
    if state_has("nudge") && state_get("nudge") {
        state_set("nudge", false);
        return "You are repeating the same action. Re-read the error output, list what you know, and try a DIFFERENT approach.";
    }
    if state_has("needs_verify") && state_get("needs_verify") {
        state_set("needs_verify", false);
        return "You just modified files. Before continuing: run the project's build and tests with bash, then fix any failures. Do not claim the task is done until they pass.";
    }
    ()
}

// --- 3) Plan pinning: keep a visible plan at the tail of the context --------
register_tool("update_plan", "Set or update the working plan (markdown checklist). Call at the start of a task and whenever steps complete or change.", #{
    type: "object",
    properties: #{ plan: #{ type: "string" } },
    required: ["plan"]
});

fn tool_update_plan(args) {
    state_set("plan", args.plan);
    "Plan updated. It will stay pinned at the tail of the context."
}

fn on_context(messages) {
    if !state_has("plan") {
        return messages;
    }
    let plan = state_get("plan");
    let filtered = messages.filter(|m|
        !(m.role == "user" && m.content.to_string().starts_with(PLAN_PIN))
    );
    let pin = #{
        role: "user",
        content: PLAN_PIN + "\n" + plan + "\n[keep this plan updated with the update_plan tool as you progress]",
        timestamp: 0
    };
    let idx = if filtered.len() > 0 { filtered.len() - 1 } else { 0 };
    filtered.insert(idx, pin);
    filtered
}
