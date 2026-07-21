# Hermes Agent gaps → pirs-claw coverage

What Hermes ships that we map onto pirs / pirs-claw.  
**Explicitly out of scope:** Singularity, Modal, Daytona terminal backends.

| Hermes capability | pirs / pirs-claw status | How |
|-------------------|-------------------------|-----|
| CLI chat | **Done** | `pirs-claw chat` |
| Full TUI | **Done** (harness) | `pirs --mode tui` |
| Telegram | **Done** | long-poll + flock single-instance; pair fail-closed |
| Discord | **Stub** | webhook + REST; not production-depth |
| Slack | **Stub** | Events webhook; not production-depth |
| WhatsApp | **Done** (thin) | Cloud API + hub.verify_token challenge |
| Signal | **Done** (thin) | `signal-cli` receive/send if installed |
| Pairing / allowlist | **Done** | `pair add/list/remove`; fail closed |
| Cross-channel session continuity | **Done** | `sessions/{channel}/{peer}.jsonl` + `*.meta.json` |
| Cron + schedule | **Done** | `--in`/`--every` durations; tick ok/fail summary |
| Local terminal | **Done** | default bash / `--exec local` |
| Docker terminal | **Done** | `--exec docker` / `docker:<image>` / `docker@ctr` (`PIRS_SANDBOX`) |
| SSH terminal | **Done** | `--exec ssh:user@host` |
| Modal / Daytona / Singularity | **Skipped** | rejected by `--exec` |
| FTS memory / recall | **Done** | SQLite FTS5 via `memory_bridge` + `pirs-claw recall` |
| Skills (agentskills-shaped) | **Done** | `skills list/show/add/usage` + `~/.pirs/skills` |
| Skill self-improve during use | **Partial** | `skill-crystallizer.rhai` on **pirs** packs |
| Subagents / parallel | **Done** | `delegate` tool on code path |
| Voice memo transcription | **Done** (hook) | `pirs-claw transcribe` + external whisper / `PIRS_CLAW_TRANSCRIBE_CMD` |
| Multi-provider models | **Done** | claw loads user `~/.pirs/config.toml` registry (same shape as pirs) |
| Honcho user modeling | **Not** | external product; not reimplemented |
| Research trajectories | **Not** | research surface; use pirs-bench / traces instead |
| Desktop app | **Not** | intentional |
| Installer one-liner | **Done** | `scripts/install.sh` ships pirs + pirs-claw + orchestrator |
| Browser / computer-use suite | **Partial** | `web-tools.rhai` pack on pirs; not full Hermes browser stack |
| Gateway process always-on | **Done** | `serve` + `scripts/pirs-claw-telegram.service` |

## Env vars (gateway)

| Channel | Required |
|---------|----------|
| telegram | `TELEGRAM_BOT_TOKEN`, allowlist peers |
| discord | `DISCORD_BOT_TOKEN`, webhook POST port `PIRS_CLAW_DISCORD_PORT` (8741) |
| slack | `SLACK_BOT_TOKEN`, port `PIRS_CLAW_SLACK_PORT` (8742) |
| whatsapp | `WHATSAPP_TOKEN`, `WHATSAPP_PHONE_NUMBER_ID`, `WHATSAPP_VERIFY_TOKEN` (hub challenge), port 8743 |
| signal | `SIGNAL_ACCOUNT`, `signal-cli` on PATH |

Dev only: `PIRS_CLAW_ALLOW_ALL=1` skips allowlist (dangerous — serve prints a loud warning).

Webhook listeners bind **`127.0.0.1` by default**. Opt-in public bind: `PIRS_CLAW_PUBLIC_BIND=1` or `PIRS_CLAW_BIND=0.0.0.0`.

Gateway messages default to **no coding tools** (recall only). Pass `--gateway-code` to enable full tools (opt-in).

## Exec backends

```bash
pirs-claw --exec local code "…"
pirs-claw --exec docker code "…"                 # image: debian:stable-slim or PIRS_SANDBOX_IMAGE
pirs-claw --exec docker:ubuntu:22.04 code "…"
pirs-claw --exec docker@my-container code "…"
pirs-claw --exec ssh:user@host code "…"
pirs-claw --exec modal …   # error: unsupported
```
