// mutation-guard.rhai — self-verifying codegen via mutation testing.
//
// The recurring failure mode of AI-written code is tests that PASS without
// actually covering the change. This pack closes that gap. After the agent
// edits Rust source and thinks it is done, it runs `cargo-mutants` scoped by
// the code graph to just the changed files and the tests that reach them. If
// any mutant SURVIVES — the suite still passes when the logic is altered — the
// tests don't really cover the change, so the pack injects a follow-up turn
// listing the survivors and makes the agent strengthen the tests, looping
// until mutation is clean or a retry cap is hit.
//
// It FAILS OPEN, never silently green: if cargo-mutants is missing, or the
// graph finds no tests reaching a changed file, it says so out loud instead of
// letting the run end as if verified.
//
// Requires `cargo install cargo-mutants`. Rust-only; mutation testing is slow,
// so results are graph-scoped to stay practical. Runs at turn end (on_follow_up)
// — never from on_tool_call — so it can't hit the hook-reentrancy path.
// Set state_set("mut_dry", true) to record the command without running it.
//
// caps: {"exec": ["cargo"], "subagents": 0}

const MAX_INJECTIONS = 3;
const MUT_OUT = ".pirs/mutants";
const PER_MUTANT_TIMEOUT = 60; // seconds, passed to cargo-mutants
const EXEC_TIMEOUT = 900; // seconds, overall cap
const MAX_FILES = 10;
const MAX_TEST_FILTER = 20;

fn is_source_rs(path) {
    if !path.ends_with(".rs") { return false; }
    if path.ends_with("_test.rs") { return false; }
    if path.contains("/tests/") { return false; }
    true
}

// Reuse the verify-impact convention for pulling a path out of a tool result.
fn impacted_path(result) {
    if "details" in result && "path" in result.details {
        return result.details.path;
    }
    let text = result.text;
    for sep in [" in ", " to "] {
        if text.contains(sep) {
            let parts = text.split(sep);
            let p = parts[parts.len() - 1];
            p.trim();
            return p;
        }
    }
    ""
}

// The changed-source set is kept as a newline-joined string (a rhai array in
// state is awkward to mutate); this keeps add/dedup trivial.
fn files_get() { state_get("mut_files") ?? "" }

fn files_list() {
    let cur = files_get();
    if cur == "" { [] } else { cur.split("\n") }
}

fn files_add(path) {
    let cur = files_get();
    for it in (if cur == "" { [] } else { cur.split("\n") }) {
        if it == path { return (); }
    }
    state_set("mut_files", if cur == "" { path } else { cur + "\n" + path });
}

// Record source edits and mark the set as needing (re)verification.
fn on_tool_result(id, name, result) {
    if result.isError { return (); }
    if name != "edit" && name != "write" && name != "ast_edit" { return (); }
    let path = impacted_path(result);
    if path == "" || !is_source_rs(path) { return (); }
    files_add(path);
    state_set("mut_state", "needs_check");
    ()
}

fn cargo_mutants_available() {
    let r = exec("cargo mutants --version", 30);
    r.code == 0
}

// Build `cargo mutants -f <file>... -- <affected tests>`, scoping the tests via
// the graph so each mutant only runs the tests that reach the changed code.
// Returns #{ cmd, scoped, untested } where `untested` lists changed files the
// graph found no reaching tests for (a finding in its own right).
fn build_plan(files) {
    let flags = "";
    let tests = [];
    let untested = [];
    for f in files {
        flags = flags + ` -f ${f}`;
        let reached = [];
        try {
            reached = graph_affected_tests(f);
        } catch (e) {
            reached = [];
        }
        if reached.len() == 0 {
            untested.push(f);
        }
        for t in reached {
            let seen = false;
            for x in tests { if x == t { seen = true; } }
            if !seen { tests.push(t); }
        }
    }
    let cmd = `cargo mutants --no-shuffle --output ${MUT_OUT} --timeout ${PER_MUTANT_TIMEOUT}${flags}`;
    let scoped = tests.len() > 0;
    if scoped {
        let run = if tests.len() > MAX_TEST_FILTER { tests.extract(0, MAX_TEST_FILTER) } else { tests };
        cmd = cmd + ` -- ${str_join(run, " ")}`;
    }
    #{ cmd: cmd, scoped: scoped, untested: untested }
}

fn on_follow_up() {
    let files = files_list();
    if files.len() == 0 { return (); }
    if (state_get("mut_state") ?? "") == "clean" { return (); }
    if files.len() > MAX_FILES { files = files.extract(0, MAX_FILES); }

    let plan = build_plan(files);

    if state_has("mut_dry") {
        state_set("mut_last_cmd", plan.cmd);
        state_set("mut_state", "clean");
        return ();
    }

    // Fail open on missing tooling: surface once, never fake a pass.
    if !cargo_mutants_available() {
        state_set("mut_state", "clean"); // stop retrying this session
        return (`Mutation testing was NOT run: cargo-mutants is not installed, so the edits to ${str_join(files, ", ")} are unverified for test coverage. Install it ('cargo install cargo-mutants') or verify the tests by hand — do not assume the change is covered.`);
    }

    let r = exec(plan.cmd, EXEC_TIMEOUT);

    if r.code == 0 {
        state_set("mut_state", "clean");
        // Even when clean, flag files the graph could not tie to any test.
        if plan.untested.len() > 0 {
            return (`Mutation testing passed for the covered code, but the graph found NO tests reaching ${str_join(plan.untested, ", ")}. Those changes are unverified — add tests that exercise them.`);
        }
        return ();
    }

    // Non-1 exit is a tooling/run error, not a survivor verdict: say so.
    if r.code != 1 {
        state_set("mut_state", "clean");
        let out = r.output;
        let tail = if out.len() > 600 { "..." + out.sub_string(out.len() - 600, 600) } else { out };
        return (`Mutation testing could not complete for ${str_join(files, ", ")} (cargo-mutants exited ${r.code}); the change is unverified:\n${tail}`);
    }

    let n = state_get("mut_injections") ?? 0;
    if n >= MAX_INJECTIONS {
        state_set("mut_state", "clean");
        return (`Mutation testing still reports surviving mutants after ${MAX_INJECTIONS} attempts in ${str_join(files, ", ")}. Stopping the loop — review the remaining survivors yourself.`);
    }
    state_set("mut_injections", n + 1);
    state_set("mut_state", "awaiting_fix");

    let out = r.output;
    let tail = if out.len() > 1200 { "..." + out.sub_string(out.len() - 1200, 1200) } else { out };
    `Mutation testing found SURVIVING mutants in ${str_join(files, ", ")} — the tests still pass when this logic is altered, so they do not actually cover the change. Add or strengthen tests to kill these mutants, then finish:\n${tail}`
}
