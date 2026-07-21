// strict-plan.rhai — *extra* denials on top of Rust `--agent-profile plan`.
//
// Rust still owns the hard readonly set (blocks bash/write/edit). This pack
// only ADDS denials when the active profile is `plan` (or when
// PIRS_STRICT_PLAN=1 forces the pack regardless of profile).
//
// Never used to *allow* tools that plan blocked — host gate runs first.
//
// Load: copy to .pirs/extensions/ or ~/.pirs/extensions/
// Pair with: pirs --agent-profile plan …

// Tools that plan-mode already allows in Rust but teams often want closed.
const EXTRA_PLAN_BLOCK = [
    "web_search",
    "web_fetch",
    "http_json",
    "browser_navigate",
    "browser_screenshot",
    "browser_cdp",
    "vision_describe",
    "computer_screenshot",
    "computer_click",
    "computer_type",
];

fn active_profile() {
    // Host API from register_core_host_apis → agent_profile(_)
    let rows = agent_profile("");
    if rows.len() > 0 {
        return `${rows[0]}`;
    }
    "default"
}

fn strict_forced() {
    let v = exec("printf %s \"$PIRS_STRICT_PLAN\"").output;
    v.trim();
    v == "1" || v == "true" || v == "yes"
}

fn on_tool_call(id, name, args) {
    let prof = active_profile();
    if prof != "plan" && !strict_forced() {
        return ();
    }
    for t in EXTRA_PLAN_BLOCK {
        if name == t {
            return #{
                block: true,
                reason: `blocked by strict-plan pack under profile '${prof}': ${name} is extra-denied (Rust plan already blocks mutate/shell; this pack closes network/browser/vision).`
            };
        }
    }
    ()
}
