# pirs-claw

Lean **personal assistant** control plane — an intentional alternative to OpenClaw and Hermes Agent **without** their channel zoo / desktop hub.

## What it is

| Feature | pirs-claw | OpenClaw / Hermes |
|---------|-----------|-------------------|
| CLI chat | yes | yes |
| Durable session file | `~/.pirs/claw/session.jsonl` | yes (heavier) |
| Schedules | `schedule add/list/tick` | full cron gateways |
| Channels | **not in v1** | 10–20+ |
| Desktop hub | no | yes (OpenClaw) |
| Skill self-evolution | no | Hermes |

Reuses `pirs-agent` + `pirs-tools` + OpenAI-compatible providers.

## Usage

```bash
cargo build -p pirs-claw

# Chat (appends to session; reloads history next time)
pirs-claw chat "remind me: standup is at 10"

# History
pirs-claw history --last 10

# Schedule a one-shot in 60s (or every N seconds)
pirs-claw schedule add --in-secs 60 "summarize my day"
pirs-claw schedule add --every 3600 "hourly pulse"
pirs-claw schedule list

# Fire due jobs:
#   tick          — dry-run: print due prompts only (does NOT mark fired)
#   tick --run    — chat each due job; mark_fired only on success (failed stay due)
pirs-claw schedule tick
pirs-claw schedule tick --run
```


State dir: `~/.pirs/claw/` (override with `--state-dir`).
