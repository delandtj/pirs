# QA — feature verification with proof

Every claim below is backed by a captured artifact in this directory. Live runs
were executed against DeepSeek (`--provider openai --base-url
https://api.deepseek.com`) using `deepseek-v4-flash` / `deepseek-v4-pro`. All
logs were scrubbed of API keys before saving (`--api-key <redacted>` in place of
the real secret); no keys or secrets are committed.

## Static gates

| Gate | Proof | Result |
|------|-------|--------|
| Full test suite | `test-suite.txt` | **491 passed, 0 failed** |
| Formatting | `fmt.txt` | clean (`cargo fmt --check`, exit 0) |
| Lint | `clippy.txt` | clean (`clippy -D warnings`, exit 0) |
| CLI surface | `cli-help.txt` | `--help` renders all flags |

Reproduce:

```sh
cargo test --workspace            # -> test-suite.txt
cargo fmt --all --check           # -> fmt.txt
cargo clippy --workspace --all-targets -- -D warnings   # -> clippy.txt
```

## Live feature runs

Each log is a full transcript of the product agent (`pirs`) exercising one
feature end to end against a real model.

| # | Feature | Proof | What it demonstrates |
|---|---------|-------|----------------------|
| 1 | Naive coding loop (pi mode) | `live/01-naive-loop.log` | The agent runs a task with no strategy — plain tool-use loop to completion. |
| 2 | `--strategy plan-exec` | `live/02-strategy-plan-exec.log` | Read-only plan phase → full-scope exec phase, `{prev}` threaded between them. |
| 3 | Oracle routing (`plan-oracle-exec`) | `live/03-oracle-routing.log` | Per-phase model override: only the critic phase runs on the stronger model. |
| 4 | Wide fan-out (`general-wide-plan-exec`) | `live/04-wide-fanout.log` | Three parallel read-only planners merge under `## Branch N` for one executor. |
| 5 | `--profile security-reviewer` | `live/05-profile-security-reviewer.log` | Provider-agnostic profile drives a review pass (no hard-pinned model). |
| 6 | Verify-and-retry gate (pass) | `live/06-gate-pass.log` | `--verify` command runs; passing verify completes the run cleanly. |
| 7 | Verify gate (fail → exit 1) | `live/07-gate-fail-exit1.log` | Exhausted attempts with a failing `--verify` exits non-zero (real exit 1). |

## Orchestration & runtime control (live)

Live runs proving the agent-loop control surface and multi-instance features.

| # | Feature | Proof | What it demonstrates |
|---|---------|-------|----------------------|
| 8 | Background jobs + waiter + monitoring | `live/08-background-jobs.log` | `bash(background:true)` → job #1; `wait_ready` confirms the server is listening; `jobs`/`job_output` monitor it; `job_kill` stops it (`running`→`killed`). |
| 9 | Steering a running turn | `live/09-steering-rpc.log` | Over `--mode rpc`, a `steer` message injected mid-turn (while a tool was executing) lands as a user message inside the running conversation and redirects the model — it abandons its 4-step plan and answers the steered question instead. |
| 10 | Fleets (orchestrator) | `live/10-fleet-orchestrator.log` | `pirs-orchestrator` daemon + `spawn` of two headless workers; `list` shows both `online`; per-instance `rpc` prompts run independently; **isolation** proven (worker-a writes `ALPHA` in its cwd, worker-b writes `BETA` in its); `status`; `stop` → `no instances`. |
| 11 | Swarm / hive coordination | `live/11-swarm-hive.log` | Two separate `pirs` processes coordinate over the shared `swarm.jsonl` blackboard (`swarm.rhai` pack): a queen `swarm_post`s two packets, a worker `swarm_claim`s + `swarm_done`s one; final blackboard shows `#1 done`, `#2 open`. Also exercises the extension re-entrancy guard. |

These map to: background-job tools (`jobs`/`job_output`/`job_kill`/`job_wait`/`wait_ready`/`job_steer`),
`agent.steer()` via the RPC `steer`/`prompt` commands, the `pirs-orchestrator`
Unix-socket fleet control (`spawn`/`list`/`status`/`stop`/`rpc`), and the
Rhai swarm pack over a shared JSONL queue.

## Incremental graph index (`--persist-graph`)

Persistent, incrementally-refreshed code-graph cache (SQLite). Skips re-parsing
unchanged files on warm starts — the scaling lever for large repos.

| # | Feature | Proof | What it demonstrates |
|---|---------|-------|----------------------|
| 12 | Incremental graph store | `live/12-persist-graph.log` | On the pirs repo: **cold** run parses all 142 files → 2065 symbols, writes `.pirs/graph.db` (0.9s); **warm** run re-parses **0**, all 142 unchanged, same 2065 symbols (0.2s, ~4.5× faster); after touching one file, **exactly 1** re-parsed, 141 unchanged — same 2065 symbols. Symbol count identical across all three = equivalence holds live. |

Correctness is also test-pinned in `crates/pirs-graph/tests/store_test.rs`:
`incremental_refresh_equals_full_parse_across_add_change_delete` asserts the
incrementally-refreshed graph is set-equivalent to a from-scratch parse across
adds, changes, and deletes; `corrupt_db_is_recreated_not_fatal` proves a garbage
cache is wiped and rebuilt rather than breaking the agent.

## Semantic code search (`--semantic`)

Natural-language retrieval over the code graph via an OpenAI-compatible
embedding service (no native ONNX dep in pirs). Embeddings are stored per-symbol
in the graph store, stamped with the model so a swap re-embeds.

| # | Feature | Proof | What it demonstrates |
|---|---------|-------|----------------------|
| 13 | `semantic_search` end to end | `live/13-semantic-search.log` | On the pirs repo, pointed at a local Ollama `all-minilm`: the tool embedded **2107 symbols**, cosine-searched a natural-language query, reranked by graph centrality, and returned ranked `file:line` hits with similarity + caller counts. |

Correctness/robustness is also test-pinned:
- `crates/pirs-ai/tests/embed_client_test.rs` — the embeddings client parses
  responses, realigns out-of-order indexes, rejects count mismatches, surfaces
  non-2xx as errors.
- `crates/pirs-graph/tests/store_test.rs::semantic_embed_store_search_and_model_guard`
  — embed/store/search ranking, the model-swap wipe guard, incremental re-embed
  on file change.
- `crates/pirs-graph/tests/semantic_fallback_test.rs` — **a bug this live run
  caught**: small-context models (all-minilm, 256 tokens) reject dense chunks.
  The fix truncates the offending chunk per-item so one oversized symbol never
  aborts the whole index; the test drives the real tool against a mock server
  that 400s over-long inputs and asserts both symbols still embed.

**Honest quality note.** The *mechanism* is proven, but retrieval *quality* is
model-bound. `all-minilm` is a tiny general-English model and is weak on code:
in the live run its top hits for a "detect file changed / staleness" query were
`diff`/`edit` symbols, not the store's actual `refresh()`/mtime logic. Semantic
search is a better *first hop* into the graph, not a substitute for it — and a
code-specific embedding model (e.g. nomic-embed-code, jina-code) is needed for
results good enough to lead exploration. That is a model choice, exposed via
`--embed-model`, not a wiring gap.

## Discovery

| Feature | Proof | What it demonstrates |
|---------|-------|----------------------|
| Strategy name resolution | `strategy-discovery.txt` | Built-ins resolve by name; unknown names fail with a helpful error. |

## Notes

- Built-in strategies (`monolithic`, `plan-exec`, `plan-critic-exec`,
  `wide-plan-exec`) are embedded in `pirs-rhai`; project `.pirs/strategies/`
  overrides and additions (e.g. `plan-oracle-exec`, `general-*`) resolve by name
  with project-then-home precedence.
- The extension packs are cataloged in `../extensions/README.md` and loaded by
  the `pirs-rhai` integration tests, counted in the 491 above.
