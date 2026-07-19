// A user-authored loop strategy, equivalent to the built-in `plan-exec`.
//
// Run it with:  pirs-bench selftest --agent --strategy-script this-file.rhai ...
//
// A strategy is data: a map with an ordered `phases` list. Each phase runs as its
// own agent sub-loop. Prompt placeholders are filled in at run time:
//   {issue}   the problem statement
//   {targets} the failing tests, as a bulleted list
//   {prev}    the previous phase's text output (e.g. the plan)
//   {verdict} on a retry, the prior gate verdict (empty on the first attempt)
//
// `scope` picks the tools the phase may use: "readonly" (read/search/navigate,
// cannot edit) or "full" (adds edit/write/shell). `persist` = reuse one context
// across attempts (like the monolithic baseline); false = a fresh context per
// phase each attempt.

#{
    name: "plan-exec-script",
    persist: false,
    phases: [
        #{
            scope: "readonly",
            system: "You are a senior engineer producing a fix plan for a bug in a real repository. \
                     Investigate with the read/search/code-graph tools to find the true root cause. \
                     Do NOT edit any files. Output only a self-contained plan naming the exact \
                     file(s), function(s), and the precise minimal change. Never touch tests.",
            prompt: "{verdict}Investigate and produce a self-contained fix plan.\n\n\
                     ## Issue\n{issue}\n\n## Tests that must pass after the fix\n{targets}\n\n\
                     Output ONLY the numbered plan. Do not edit.",
        },
        #{
            scope: "full",
            system: "You are fixing a bug by executing an APPROVED plan. Make the edits the plan \
                     specifies, and nothing more. Fix source, never tests. You may run the tests. \
                     When the target tests pass and nothing else broke, stop.",
            prompt: "Execute this approved fix plan in the repository.\n\n\
                     ## Issue\n{issue}\n\n## Tests that must pass\n{targets}\n\n\
                     ## Approved plan\n{prev}\n\nMake the edits now and verify.",
        },
    ],
}
