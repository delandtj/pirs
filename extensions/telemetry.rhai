// telemetry.rhai — local, metadata-only usage telemetry.
//
// Off by default like every pack in this catalog: nothing runs until you
// copy this into `.pirs/extensions/` or `~/.pirs/extensions/`. Appends one
// JSON line per turn/tool-call/session to `~/.pirs/telemetry.jsonl` — counts,
// durations, token usage, enum-ish state (stop reason, error yes/no) — and
// deliberately never the content-bearing fields `on_event` also carries
// (assistant `text`, tool `args`, `errorMessage`). That's enforced by
// construction here (each record below is a hand-picked field list, not a
// pass-through of the event map).
//
// Prefer CLI `--trace` for a full flight recorder (agent events + strategy
// phases). This pack is the lightweight, content-free always-on option.
//
// This is deliberately a different, narrower thing than `audit-log.rhai`,
// which exists precisely to capture full tool args/results for local
// debugging — pick audit-log when you want the content, telemetry when you
// want the shape of a session without it.

fn telemetry_path() {
    let home = exec("printf %s \"$HOME\"").output;
    home.trim();
    `${home}/.pirs/telemetry.jsonl`
}

fn write_record(fields) {
    fields.ts = now_millis();
    // Host-provided session identity (see pirs_rhai::set_session_meta).
    fields.sessionId = session_id();
    // Active model (updated per strategy phase when plan-model is used).
    fields.model = agent_model();
    fs_append(telemetry_path(), fields.to_json() + "\n");
}

fn on_event(ty, data) {
    if ty == "turn_end" {
        // `model` is filled by the host on turn_end (AssistantMessage.model);
        // session-level agent_model() is also stamped in write_record.
        write_record(#{
            kind: "turn",
            stopReason: data.stopReason,
            numToolResults: data.numToolResults,
            inputTokens: data.inputTokens,
            cacheReadTokens: data.cacheReadTokens,
            outputTokens: data.outputTokens,
            turnModel: data.model,
        });
    } else if ty == "tool_execution_end" {
        write_record(#{
            kind: "tool_call",
            tool: data.name,
            isError: data.isError,
        });
    } else if ty == "compaction_end" {
        write_record(#{
            kind: "compaction",
            reason: data.reason,
            aborted: data.aborted,
        });
    } else if ty == "agent_end" {
        write_record(#{
            kind: "session_end",
            numMessages: data.numMessages,
            inputTokens: data.inputTokens,
            cacheReadTokens: data.cacheReadTokens,
            outputTokens: data.outputTokens,
            totalTokens: data.totalTokens,
        });
    }
    ()
}
