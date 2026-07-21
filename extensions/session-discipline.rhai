// session-discipline.rhai — steer use of ask_user + todo (policy only).
//
// Tools live in Rust (ask_user, todo). This pack only nudges the agent when
// sessions thrash without checklist updates or when ambiguity is likely.
//
// Load: .pirs/extensions/ or ~/.pirs/extensions/

fn on_tool_call(id, name, args) {
    if name == "todo" {
        state_set("todo_used", true);
        state_set("idle_since_todo", 0);
    } else if name == "ask_user" {
        state_set("ask_used", true);
    } else {
        let idle = if state_has("idle_since_todo") { state_get("idle_since_todo") } else { 0 };
        state_set("idle_since_todo", idle + 1);
    }
    if name == "edit" || name == "edit_block" || name == "write" {
        state_set("mutate_count", (state_get("mutate_count") ?? 0) + 1);
    }
    ()
}

fn on_tool_result(id, name, result) {
    if name == "todo" && !result.isError {
        state_set("todo_used", true);
    }
    ()
}

fn on_steering() {
    let out = [];
    let mutates = if state_has("mutate_count") { state_get("mutate_count") } else { 0 };
    let todo_used = if state_has("todo_used") { state_get("todo_used") } else { false };
    let idle = if state_has("idle_since_todo") { state_get("idle_since_todo") } else { 0 };

    if mutates >= 2 && !todo_used && !state_has("todo_steer_sent") {
        state_set("todo_steer_sent", true);
        out.push("Use the `todo` tool to track multi-step work (add/list/update). Keep the checklist current after each meaningful step.");
    }
    if idle >= 8 && todo_used && !state_has("todo_refresh_steer") {
        state_set("todo_refresh_steer", true);
        out.push("Update your `todo` list (mark completed / add remaining work) before more edits.");
    }
    if mutates >= 4 && !state_has("ask_used") && !state_has("ask_steer_sent") {
        state_set("ask_steer_sent", true);
        out.push("If requirements are ambiguous, call `ask_user` with clear options (id + label) instead of guessing.");
    }
    out
}
