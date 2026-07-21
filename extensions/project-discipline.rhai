// project-discipline.rhai — prefer the shared `project` tool over inventing shell.
//
// After successful bash that looks like a project command, steer the agent
// toward project(action: "…"). Complements Rust-side shell hints.
//
// Load: copy to ~/.pirs/extensions/ or .pirs/extensions/
// Claw: enabled when extensions load (default on for chat/code; --no-extensions to skip).

fn on_tool_result(id, name, result) {
    if name != "bash" || result.isError {
        return ();
    }
    // Soft steering is handled by host bash hints; this pack records usage for crystallize.
    let cur = if state_has("project_shell_hits") { state_get("project_shell_hits") } else { 0 };
    state_set("project_shell_hits", cur + 1);
    ()
}

fn on_context(messages) {
    // Nudge once per session after repeated shell hits.
    let hits = if state_has("project_shell_hits") { state_get("project_shell_hits") } else { 0 };
    if hits < 2 || state_has("project_nudge_sent") {
        return messages;
    }
    state_set("project_nudge_sent", true);
    // Leave messages unchanged; host system prompt already lists project commands.
    // Steering is more effective via on_steering.
    messages
}

fn on_steering() {
    let hits = if state_has("project_shell_hits") { state_get("project_shell_hits") } else { 0 };
    if hits >= 2 && !state_has("project_steer_done") {
        state_set("project_steer_done", true);
        return [
            "Prefer the `project` tool (list/test/lint/typecheck/build) over inventing cargo/npm/pytest shell commands. Use project(action: \"packages\") in monorepos."
        ];
    }
    []
}
