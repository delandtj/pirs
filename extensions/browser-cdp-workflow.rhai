// browser-cdp-workflow.rhai — usage policy for pure-Rust browser_cdp (not the client).
//
// The chromiumoxide tool stays in Rust. This pack steers multi-step browser work
// and blocks reckless eval/screenshot spam patterns lightly.
//
// Load when doing web automation sessions.

fn on_tool_call(id, name, args) {
    if name != "browser_cdp" {
        return ();
    }
    let action = if args.contains("action") { `${args.action}` } else { "" };
    let n = (state_get("cdp_calls") ?? 0) + 1;
    state_set("cdp_calls", n);

    // Soft thrash: many evals without navigation.
    if action == "eval" {
        let evals = (state_get("cdp_evals") ?? 0) + 1;
        state_set("cdp_evals", evals);
        if evals >= 6 && (state_get("cdp_gotos") ?? 0) == 0 {
            state_set("cdp_nudge", "nav");
        }
    }
    if action == "goto" {
        state_set("cdp_gotos", (state_get("cdp_gotos") ?? 0) + 1);
        state_set("cdp_evals", 0);
    }
    if action == "close" {
        state_set("cdp_calls", 0);
        state_set("cdp_evals", 0);
        state_set("cdp_gotos", 0);
    }
    ()
}

fn on_steering() {
    if state_has("cdp_nudge") && state_get("cdp_nudge") == "nav" && !state_has("cdp_nav_steer") {
        state_set("cdp_nav_steer", true);
        return [
            "browser_cdp recipe: connect → goto(url) → content/screenshot → click/type as needed → close. Prefer content over many evals; call close when done."
        ];
    }
    if (state_get("cdp_calls") ?? 0) == 1 && !state_has("cdp_intro_steer") {
        state_set("cdp_intro_steer", true);
        return [
            "For browser_cdp: set PIRS_BROWSER_CDP_URL or let Chromium auto-launch; always close when finished to free the session."
        ];
    }
    []
}
