# Hero Shrimp → pirs transfer map

Survey of `~/hero/code/hero_shrimp` for ideas worth reusing in pirs vs
explicitly **skipping** (bloat or already covered). Goal: lean products
**pirs-work** (coding agent) and **pirs-claw** (personal assistant), not a
shrimp fork.

| Capability | Verdict | Rationale |
|------------|---------|-----------|
| **Independent review / done-gates** (stronger model reviews every diff; vacuous-green refuse) | **Useful** | Core shrimp insight: cheap executor + blocking reviewer. pirs already has `review-gate.rhai`, `verify-guard.rhai`, stop-gate in weak pack — **prefer packs + `--plan-model`**, not a second review engine. |
| **Durable goals / runs** (goal dir + resume after restart) | **Useful (light)** | pirs has `goal.rhai`, `runs.rhai`, sessions JSONL. Claw/work should **reuse session files**, not shrimp’s goals/ RPC surface. |
| **Schedules / cron** | **Useful (one path)** | Hermes/OpenClaw/shrimp all treat cron as load-bearing for “always-on”. **pirs-claw** gets a **minimal schedule store + tick**, not Hero service lifecycle. |
| **Multi-provider routing** | **Useful — already in pirs** | shrimp.yml backends ≈ `~/.pirs/config.toml` `[[backends]]`/`[[models]]` + `RoutingProvider`. **Do not reimplement.** |
| **Channels** (Telegram, WhatsApp, …) | **Skip matrix; optional one later** | OpenClaw’s 20+ channels and shrimp_telegram are product bloat for overnight scope. Claw ships **CLI-first**; one adapter is enough if ever added. |
| **Wire-log / crypto audit** (Ed25519 hash-chain) | **Skip (defer)** | Strong for compliance; heavy and shrimp-specific. pirs has audit-log pack + `--trace` JSONL without crypto. |
| **Web UI** (Dioxus operator UI) | **Skip** | Full stack; pirs already has REPL/TUI/`--serve`. No second UI framework. |
| **OpenRPC / Hero service manager** | **Skip** | Ops surface for Hero fleet, not needed for standalone pirs-work/claw. |
| **OTEL / full observability stack** | **Skip** | pirs has `--trace` + session stats; full OTEL is shrimp_otel bloat for now. |
| **USD spend caps (persistent)** | **Useful idea — pack exists** | `spend-caps.rhai` already; wire into claw defaults if needed, don’t port meter code. |
| **Skill evolution / RL trajectories** | **Skip** | Research/eval surface; out of product scope. |
| **Council / multi-agent debate** | **Skip for default** | pirs has arena/relay packs; not default for work/claw. |

## OpenClaw / Hermes (shape only)

| Idea | For pirs-claw? |
|------|----------------|
| Always-on gateway + many channels | **No** — CLI + optional schedule |
| Session memory across restarts | **Yes** — file-backed JSONL |
| Cron / scheduled prompts | **Yes** — lean store + `tick` |
| Skill self-improvement loop | **No** for v1 |
| Desktop hub / mobile | **No** |

## Product split

| Product | Role | Peers |
|---------|------|--------|
| **pirs-work** | Terminal coding agent on a repo | Claude Code, Codex, Qoder, Kimi Code, qwen-code |
| **pirs-claw** | Personal assistant control plane | OpenClaw, Hermes Agent (thinner) |
| **pirs** (main) | Full harness / TUI / strategies / bench | pi, etc. |

Neither product vendors shrimp source; both depend on `pirs-agent` / `pirs-tools` / `pirs-ai` / registry.
