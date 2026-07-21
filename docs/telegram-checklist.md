# Telegram production checklist (pirs-claw)

Use this before putting `pirs-claw serve --channel telegram` on a machine that
reaches real users.

## Required

1. **Bot token** — set `TELEGRAM_BOT_TOKEN` (or `PIRS_TELEGRAM_BOT_TOKEN`) in the
   environment or `~/.pirs/secrets.env`. Serve fails closed without it.
2. **Pairing allowlist** — add chat ids (or user ids) before serving:
   ```bash
   pirs-claw pair add YOUR_CHAT_ID
   pirs-claw pair list
   ```
   Empty allowlist → serve exits with an error (fail closed).  
   Dev-only bypass: `PIRS_CLAW_ALLOW_ALL=1` prints a loud warning and is **not**
   for production.
3. **Single instance** — Telegram `getUpdates` long-poll is exclusive. Run only
   **one** `pirs-claw serve --channel telegram` process per bot token (no second
   host, no concurrent webhook on the same bot).
4. **Webhook channels bind localhost by default** — Telegram uses long-poll (no
   public port). For other channels, webhooks listen on `127.0.0.1` unless you
   set `PIRS_CLAW_PUBLIC_BIND=1` or `PIRS_CLAW_BIND=0.0.0.0` behind a reverse
   proxy / firewall.

## Recommended

5. **Coding tools off on gateway** — default is chat-safe (recall only). Only
   enable `--gateway-code` for trusted allowlisted peers on a trusted host.
6. **systemd user unit** — copy
   [`scripts/pirs-claw-telegram.service`](../scripts/pirs-claw-telegram.service):
   ```bash
   mkdir -p ~/.config/systemd/user
   cp scripts/pirs-claw-telegram.service ~/.config/systemd/user/
   systemctl --user daemon-reload
   systemctl --user enable --now pirs-claw-telegram.service
   ```
   The process also takes a **flock** on `~/.pirs/claw/locks/telegram.lock` so a
   second serve fails immediately instead of fighting getUpdates.
7. **State dir** — default `~/.pirs/claw/`; sessions under
   `sessions/telegram/<chat_id>.jsonl` (+ `*.meta.json`); allowlist at
   `allowlist.txt`.
8. **Secrets mode** — keep `secrets.env` mode `600`.
9. **Model registry** — optional `~/.pirs/config.toml` `[[backends]]` /
   `[[models]]` (same as `pirs`); claw loads **user** config only.

## Verify before go-live

```bash
# Fail closed without pair/token
pirs-claw serve --channel telegram   # expect error

pirs-claw pair add <id>
# with token in env:
pirs-claw serve --channel telegram   # long-poll starts
```

See also [HERMES-GAPS.md](HERMES-GAPS.md) and [pirs-claw.md](pirs-claw.md).
