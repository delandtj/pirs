// blame.rhai — line-level provenance. When HEAD moves during a turn (the
// agent committed), annotate the new commit(s) with (session, turn, model) via
// git notes. `pirs blame <file:line>` reads them back.
//
// A single turn can produce SEVERAL commits; annotating only the final HEAD
// would leave the intermediate ones without provenance, so we walk the whole
// prev..HEAD range and tag each. We also set notes.rewriteRef once, otherwise
// a later rebase/amend/cherry-pick silently drops the notes and the provenance
// is lost.
//
// Note keys are deliberately grep-friendly: pirs-session=, pirs-turn=,
// pirs-model=.

const MAX_COMMITS = 50; // cap when walking a turn's new commits (rewrite safety)

// Make notes follow history rewrites. Without rewriteRef, `git rebase`/`amend`
// carry the commit but not its note, erasing provenance. Idempotent; run once.
fn ensure_notes_follow_rewrites() {
    if state_has("notes_cfg") { return (); }
    exec("git config notes.rewriteRef refs/notes/commits", 10);
    exec("git config notes.rewriteMode concatenate", 10);
    state_set("notes_cfg", true);
}

fn annotate(sha, turn) {
    let note = `pirs-session=${session_id()} pirs-turn=${turn} pirs-model=${agent_model()}`;
    exec(`git notes add -f -m "${note}" ${sha}`, 10);
}

fn on_event(ty, data) {
    if ty != "turn_end" { return (); }
    ensure_notes_follow_rewrites();

    let turn = if state_has("turn") { state_get("turn") } else { 0 };
    let turn = turn + 1;
    state_set("turn", turn);

    let head = exec("git rev-parse HEAD", 10);
    if head.code != 0 { return (); }
    let cur = head.output;
    cur.trim();  // rhai trim() mutates in place; assigning its result gives ()
    if cur == "" { return (); }
    let prev = if state_has("head") { state_get("head") } else { "" };
    if cur == prev { return (); }
    state_set("head", cur);

    // Collect every commit made since we last looked, newest-first. On the
    // first turn (no prior head) we only tag the current HEAD rather than
    // walking the entire pre-existing history.
    let shas = [];
    if prev == "" {
        shas.push(cur);
    } else {
        let rl = exec(`git rev-list ${prev}..${cur}`, 10);
        if rl.code == 0 {
            let out = rl.output;
            out.trim();
            if out != "" {
                for line in out.split("\n") {
                    line.trim();
                    if line != "" { shas.push(line); }
                }
            }
        }
        // Divergent history / rewrite: rev-list may be empty even though HEAD
        // moved. Fall back to tagging HEAD so provenance is never skipped.
        if shas.len() == 0 { shas.push(cur); }
    }
    if shas.len() > MAX_COMMITS { shas = shas.extract(0, MAX_COMMITS); }

    for s in shas { annotate(s, turn); }
    ()
}
