// auto-checkpoint.rhai — thin policy: after successful mutate tools, snapshot
// via the **Rust** checkpoint path (host `checkpoint_create` → same layout as
// the `checkpoint` tool and `/checkpoint`).
//
// Prefer this over reimplementing snapshots in Rhai. Restore with:
//   - agent tool: checkpoint action=restore
//   - slash: /checkpoint restore [id]
//   - pack command: /auto-checkpoint-restore [id]
//
// Does **not** replace core checkpoints — it only auto-triggers them.
// Optional: do not load alongside file-checkpoints/stash-checkpoint if you
// want a single snapshot policy (those packs use separate layouts/commands).
//
// Load: copy to .pirs/extensions/ or ~/.pirs/extensions/
// Requires host APIs from register_core_host_apis() (always on in pirs / pirs-claw).

fn is_mutate(name) {
    name == "write" || name == "edit" || name == "edit_block" || name == "safe_edit" || name == "ast_edit"
}

fn on_tool_result(id, name, result) {
    if !is_mutate(name) {
        return ();
    }
    // Skip failed tool results (isError is set by the host map).
    if result.isError == true {
        return ();
    }
    // Host API → pirs_tools::create_checkpoint (core, not pack-local files).
    // Pass cwd explicitly so snapshot lands in the workspace even if process cwd races.
    let pwd = exec("pwd", 5);
    let cwd = `${pwd.output}`.trim();
    let rows = checkpoint_create(`after ${name}|${cwd}`);
    if rows.len() > 0 {
        let first = `${rows[0]}`;
        if first.starts_with("error:") {
            // Soft-fail: never block the agent on snapshot issues.
            return ();
        }
        state_set("last_auto_checkpoint", first);
        state_set("last_auto_cwd", cwd);
    }
    ()
}

// Command names must match `fn cmd_<name>` with underscores (no hyphens).
register_command(
    "auto_checkpoint_list",
    "List core checkpoints (same index as tool checkpoint / /checkpoint)"
);

fn cmd_auto_checkpoint_list(args) {
    let cwd = if state_has("last_auto_cwd") {
        `${state_get("last_auto_cwd")}`
    } else {
        let p = exec("pwd", 5).output;
        p.trim();
        p
    };
    let rows = checkpoint_list(cwd);
    if rows.len() == 0 {
        return "no core checkpoints yet (mutate a file with this pack loaded, or /checkpoint create)";
    }
    let out = "core checkpoints:\n";
    for r in rows {
        out = out + `- ${r}\n`;
    }
    let last = if state_has("last_auto_checkpoint") {
        state_get("last_auto_checkpoint")
    } else {
        ""
    };
    if `${last}` != "" {
        out = out + `last auto: ${last}\n`;
    }
    out
}

register_command(
    "auto_checkpoint_restore",
    "Restore core checkpoint by id (default: last auto, else latest)"
);

fn cmd_auto_checkpoint_restore(args) {
    let id = args;
    id.trim();
    if id == "" {
        if state_has("last_auto_checkpoint") {
            id = `${state_get("last_auto_checkpoint")}`;
        }
    }
    let cwd = if state_has("last_auto_cwd") {
        `${state_get("last_auto_cwd")}`
    } else {
        let p = exec("pwd", 5).output;
        p.trim();
        p
    };
    let rows = checkpoint_restore(`${id}|${cwd}`);
    if rows.len() == 0 {
        return "restore returned empty";
    }
    `${rows[0]}`
}
