// Parallel dispatch: N read-only planners investigate the SAME bug from different
// angles CONCURRENTLY, their findings are merged, then one fresh executor acts on
// the combined plan. The SoulForge "dispatch" pattern — fan out research, then a
// single edit.
//
// The `parallel` array is the fan-out: every branch runs at once (read-only by
// contract, so they can't race on the tree). `join: "concat"` stitches each
// branch's plan under a `## Branch N` heading into the executor's `{prev}`; use
// `join: "first"` to keep only the primary branch.
//
//   pirs --strategy wide-plan-exec "diagnose and fix the race in worker.rs"
// or in the bench:  pirs-bench selftest --agent --model deepseek-v4-flash \
//       --strategy-script this-file.rhai --trace run.jsonl ...
//
// In the trace, the three planner phase.start events share one attempt and their
// [start,end] intervals overlap — that overlap is the proof they ran in parallel.

#{
    name: "wide-plan-exec",
    persist: false,
    phases: [
        #{
            parallel: [
                #{
                    scope: "readonly",
                    system: "You are a senior engineer producing a fix plan. Investigate with the \
                             read/search tools and output a self-contained plan. Do NOT edit.",
                    prompt: "Focus on the failing assertion: what value is produced vs expected, \
                             and which function computes it.\n\nInvestigate and produce a \
                             self-contained fix plan.\n\n## Issue\n{issue}\n\n## Tests\n{targets}\n\n\
                             Output ONLY the plan.",
                },
                #{
                    scope: "readonly",
                    system: "You are a senior engineer producing a fix plan. Investigate with the \
                             read/search tools and output a self-contained plan. Do NOT edit.",
                    prompt: "Focus on the most recently changed or most complex code path touched \
                             by the failing test.\n\nInvestigate and produce a self-contained fix \
                             plan.\n\n## Issue\n{issue}\n\n## Tests\n{targets}\n\nOutput ONLY the plan.",
                },
                #{
                    scope: "readonly",
                    system: "You are a senior engineer producing a fix plan. Investigate with the \
                             read/search tools and output a self-contained plan. Do NOT edit.",
                    prompt: "Focus on boundary/edge handling (empty, zero, off-by-one, sign) in the \
                             code under test.\n\nInvestigate and produce a self-contained fix \
                             plan.\n\n## Issue\n{issue}\n\n## Tests\n{targets}\n\nOutput ONLY the plan.",
                },
            ],
            join: "concat",
        },
        #{
            scope: "full",
            system: "You execute an APPROVED plan. Make the minimal edits it specifies. Fix source, \
                     never tests. Run the tests; when they pass and nothing else broke, stop.",
            prompt: "Execute this plan, synthesized from parallel investigations.\n\n## Issue\n\
                     {issue}\n\n## Tests\n{targets}\n\n## Merged plan\n{prev}\n\nMake the edits now \
                     and verify.",
        },
    ],
}
