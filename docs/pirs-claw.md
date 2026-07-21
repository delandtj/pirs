# pirs-claw

**Daily agent** over the pirs core: repo work, chat, schedules, and a **multi-channel gateway**.  
Equal peer product to the `pirs` harness (see [PRODUCTS.md](PRODUCTS.md)).  
Hermes gap map: [HERMES-GAPS.md](HERMES-GAPS.md).

## Modes

| Mode | Command | Notes |
|------|---------|--------|
| **Code** | `pirs-claw -C repo "…"` / `code` | plan-exec + plan-model + delegate |
| **Chat** | `pirs-claw chat "…"` | durable session + FTS memory |
| **Schedule** | `schedule add/list/tick` | `--in`/`--every` as `30s`/`5m`/`2h`/`1d`; tick summary |
| **Gateway** | `serve --channel <name>` | telegram (+ discord/slack/whatsapp/signal stubs) |
| **Sessions** | `sessions` | multi-key `(channel, peer)` + meta |
| **Recall / skills** | `recall`, `skills list\|show\|add\|usage` | memory + `~/.pirs/skills` |
| **Pair** | `pair list\|add\|remove` | gateway allowlist |
| **Voice** | `transcribe <file>` | external whisper / custom cmd |

## Model registry

Same shape as the harness — **user** `~/.pirs/config.toml` only (no project-trust path):

```toml
[[backends]]
name = "dashscope"
kind = "openai_compatible"
base_url = "https://coding-intl.dashscope.aliyuncs.com/v1"
api_key_env = "DASHSCOPE_API_KEY"

[[models]]
alias = "qwen-plus"
serve = [{ backend = "dashscope", model = "qwen3.5-plus" }]
```

```bash
pirs-claw --model qwen-plus chat "hi"
```

Keys resolve from backend `api_key_env` / secrets.env. Unregistered model names fall back to env-key provider detection.

## Exec backends (Hermes local/Docker/SSH)

```bash
pirs-claw --exec local code "…"
pirs-claw --exec docker code "…"
pirs-claw --exec docker:ubuntu:22.04 code "…"
pirs-claw --exec docker@running-ctr code "…"
pirs-claw --exec ssh:user@host code "…"
```

**Not supported:** Modal, Daytona, Singularity.

## Sessions

Each `(channel, peer)` has its own transcript + sidecar meta:

```text
~/.pirs/claw/sessions/{channel}/{peer}.jsonl
~/.pirs/claw/sessions/{channel}/{peer}.meta.json   # last_active, message_count
```

CLI chat uses `cli/local`. Legacy `session.jsonl` is migrated once into
`sessions/cli/local.jsonl`. List with `pirs-claw sessions`.

## Schedule

```bash
pirs-claw schedule add --in 30s --every 1h "pulse"
pirs-claw schedule add --in 0 --deliver telegram:CHAT_ID "ping me"
pirs-claw schedule tick          # dry-run: list due, do not fire
pirs-claw schedule tick --run    # fire; [tick summary] ok=N failed=M
```

Failed fires stay due for retry. Deliver targets: `cli`, `telegram:<id>`, etc.

## Gateway

```bash
# Pairing (required unless PIRS_CLAW_ALLOW_ALL=1 — prints a danger warning)
pirs-claw pair add YOUR_CHAT_ID
pirs-claw pair list
pirs-claw pair remove YOUR_CHAT_ID

export TELEGRAM_BOT_TOKEN=…
pirs-claw serve --channel telegram
# acquires flock on ~/.pirs/claw/locks/telegram.lock (single getUpdates instance)

# Optional: allow coding tools from messaging (default off)
pirs-claw serve --channel telegram --gateway-code
```

Production checklist: [telegram-checklist.md](telegram-checklist.md).  
systemd example: [../scripts/pirs-claw-telegram.service](../scripts/pirs-claw-telegram.service).

Webhook-style channels (discord/slack/whatsapp) listen on **127.0.0.1** by default.  
Public bind only if you set `PIRS_CLAW_PUBLIC_BIND=1` or `PIRS_CLAW_BIND=0.0.0.0`.

**WhatsApp:** Meta hub challenge uses `WHATSAPP_VERIFY_TOKEN` (or `PIRS_WHATSAPP_VERIFY_TOKEN`) on GET verify.

Other channels: `discord`, `slack`, `whatsapp`, `signal` — stubs for Hermes set; production depth is Telegram-first (Slack/Discord intentionally shallow).

## Skills & memory

```bash
pirs-claw skills              # list
pirs-claw skills show NAME
pirs-claw skills add ./path   # install into ~/.pirs/skills
pirs-claw skills usage
pirs-claw recall "keyword"
```

- Skills: `~/.pirs/skills/**/SKILL.md` (frontmatter `name` / `description`)
- Memory: `~/.pirs/claw/memory.db` (FTS5)
- Crystallize skills after coding runs: load `skill-crystallizer.rhai` on **pirs**

## Install

```bash
# from release bundle
curl -fsSL …/scripts/install.sh | sh   # installs pirs + pirs-claw + pirs-orchestrator

# from source
cargo install --path crates/pirs-claw
```

## Intentionally not

| Skip | Why |
|------|-----|
| Modal / Daytona / Singularity | Explicit exclusion |
| Desktop Work suite | Different product class |
| Honcho / full skill self-evolution product | Hermes research moat; partial via packs |
| OpenClaw-scale channel matrix beyond Hermes set | Five channels + CLI is the Hermes set |
| Deep Slack / Discord productization | Telegram + WA verify first |

## Interactive harness

```bash
pirs --mode tui --strategy plan-exec --model qwen3.5-plus --plan-model deepseek-v4-pro
```

Keys: `~/.pirs/secrets.env`.
