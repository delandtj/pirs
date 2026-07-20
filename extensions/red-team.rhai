// red-team.rhai — after edits, a fresh-context adversary attacks the changes.
//
// Mechanically overlaps with review-gate.rhai: same trigger (edit/write
// success), same shape (a fresh sub-agent reviews the diff at on_follow_up).
// The difference is framing and strictness, not redundancy — this one hunts
// for bugs/edge-cases with a pure LGTM-or-not verdict and only nudges (the
// model can proceed even if it disagrees); review-gate.rhai judges the diff
// against the ORIGINAL REQUEST specifically (catches test-gaming, wrong
// behavior) and can hard-block completion via on_should_stop. Combining both
// is a legitimate "two lenses, more confidence" choice, but it does mean
// every edit turn pays for two separate sub-agent review calls — know that
// going in, it's not free.

const ATTACK_PROMPT = "You are an adversarial reviewer. Attack this diff: find bugs, edge cases, race conditions, broken invariants. Be brutal and specific. If you find nothing wrong, say LGTM explicitly.";

fn on_tool_result(id, name, result) {
    if (name == "edit" || name == "write") && !result.isError {
        state_set("needs_attack", true);
    }
    ()
}

fn on_follow_up() {
    if !state_has("needs_attack") || !state_get("needs_attack") {
        return ();
    }
    state_set("needs_attack", false);
    let diff = exec("git diff HEAD 2>/dev/null | head -c 12000", 15).output;
    let dtrim = diff;
    dtrim.trim();
    if dtrim == "" {
        return ();
    }
    let findings = run_subagent(`${ATTACK_PROMPT}\n\n${diff}`);
    if findings.contains("LGTM") {
        return ();
    }
    [`An adversarial reviewer found issues in your changes:\n${findings}\n\nFix the real ones (push back on wrong ones), then finish.`]
}
