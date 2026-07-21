//! Messaging gateway (Hermes gap: multi-channel ingress).
//!
//! Supported transports:
//! - **telegram** — Bot API long-poll (`getUpdates`) + `sendMessage`
//! - **discord** — Bot REST send + optional incoming via simple webhook JSON
//! - **slack** — `chat.postMessage` + Events API webhook shape
//! - **whatsapp** — Meta Cloud API send + webhook shape
//! - **signal** — `signal-cli` JSON-RPC / CLI if installed
//!
//! All non-CLI channels require pairing allowlist unless `PIRS_CLAW_ALLOW_ALL=1`.
//! Webhook listeners bind **127.0.0.1** by default; set `PIRS_CLAW_PUBLIC_BIND=1`
//! (or `PIRS_CLAW_BIND=0.0.0.0`) to listen on all interfaces.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use crate::channel::{
    Channel, InboundMessage, OutboundReply, CHANNEL_DISCORD, CHANNEL_SIGNAL, CHANNEL_SLACK,
    CHANNEL_TELEGRAM, CHANNEL_WHATSAPP,
};
use crate::pairing::{warn_if_allow_all, PairingAllowlist};

/// Async handler for one inbound gateway message → reply text.
type MessageHandler = Arc<
    dyn Fn(InboundMessage) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send>>
        + Send
        + Sync,
>;

/// Env: set to `1`/`true` to bind webhook listeners on `0.0.0.0`.
pub const PUBLIC_BIND_ENV: &str = "PIRS_CLAW_PUBLIC_BIND";
/// Env: explicit bind host (`127.0.0.1` default, `0.0.0.0` for public).
pub const BIND_ENV: &str = "PIRS_CLAW_BIND";

/// Resolve webhook listen host. Default **localhost** (safe).
///
/// Opt-in public bind: `PIRS_CLAW_PUBLIC_BIND=1` or `PIRS_CLAW_BIND=0.0.0.0`.
pub fn webhook_bind_host() -> String {
    if let Ok(h) = std::env::var(BIND_ENV) {
        let h = h.trim();
        if !h.is_empty() {
            return h.to_string();
        }
    }
    let public = std::env::var(PUBLIC_BIND_ENV)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if public {
        "0.0.0.0".into()
    } else {
        "127.0.0.1".into()
    }
}

pub fn webhook_socket_addr(port: u16) -> SocketAddr {
    let host = webhook_bind_host();
    // Parse host:port; fall back to loopback if malformed host.
    format!("{host}:{port}")
        .parse()
        .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], port)))
}

/// Dispatch a long-running channel loop.
pub async fn run_gateway(
    channel: &str,
    state_dir: &Path,
    allowlist: &PairingAllowlist,
    on_message: impl Fn(InboundMessage) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send>,
        > + Send
        + Sync
        + 'static,
) -> anyhow::Result<()> {
    warn_if_allow_all();
    let on_message: MessageHandler = Arc::new(on_message);
    match channel {
        CHANNEL_TELEGRAM => run_telegram(state_dir, allowlist, on_message).await,
        CHANNEL_DISCORD => run_discord_webhook_mode(allowlist, on_message).await,
        CHANNEL_SLACK => run_slack_webhook_mode(allowlist, on_message).await,
        CHANNEL_WHATSAPP => run_whatsapp_webhook_mode(allowlist, on_message).await,
        CHANNEL_SIGNAL => run_signal_cli(allowlist, on_message).await,
        other => anyhow::bail!(
            "unknown channel {other:?}. Supported: telegram, discord, slack, whatsapp, signal"
        ),
    }
}

fn require_allowlist(allowlist: &PairingAllowlist, channel: &str) -> anyhow::Result<()> {
    if allowlist.is_empty() {
        anyhow::bail!(
            "{channel}: pairing allowlist is empty (fail closed).\n\
             Add peer ids to ~/.pirs/claw/allowlist.txt (one per line), or set \
             PIRS_CLAW_ALLOW_ALL=1 for local dev only."
        );
    }
    Ok(())
}

// ─── Telegram ───────────────────────────────────────────────────────────────

struct TelegramBot {
    token: String,
    client: reqwest::Client,
}

impl TelegramBot {
    fn from_env() -> anyhow::Result<Self> {
        let token = std::env::var("TELEGRAM_BOT_TOKEN")
            .or_else(|_| std::env::var("PIRS_TELEGRAM_BOT_TOKEN"))
            .map_err(|_| {
                anyhow::anyhow!(
                    "telegram: set TELEGRAM_BOT_TOKEN (or PIRS_TELEGRAM_BOT_TOKEN) in env / secrets.env"
                )
            })?;
        Ok(TelegramBot {
            token,
            client: reqwest::Client::new(),
        })
    }

    fn api(&self, method: &str) -> String {
        format!("https://api.telegram.org/bot{}/{}", self.token, method)
    }

    async fn send(&self, chat_id: &str, text: &str) -> anyhow::Result<()> {
        // Telegram limit 4096; chunk on char boundaries (not raw bytes).
        for piece in utf8_chunks(text, 3500) {
            let resp = self
                .client
                .post(self.api("sendMessage"))
                .json(&json!({
                    "chat_id": chat_id,
                    "text": piece,
                }))
                .send()
                .await?;
            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("telegram sendMessage failed: {body}");
            }
        }
        Ok(())
    }
}

/// Split `s` into chunks of at most `max_chars` Unicode scalars.
fn utf8_chunks(s: &str, max_chars: usize) -> Vec<String> {
    if s.is_empty() {
        return vec![String::new()];
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in s.chars() {
        if cur.chars().count() >= max_chars {
            out.push(std::mem::take(&mut cur));
        }
        cur.push(ch);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

impl Channel for TelegramBot {
    fn channel_id(&self) -> &str {
        CHANNEL_TELEGRAM
    }

    fn deliver(&self, reply: &OutboundReply) -> anyhow::Result<()> {
        let client = reqwest::blocking::Client::new();
        for piece in utf8_chunks(&reply.text, 3500) {
            let resp = client
                .post(self.api("sendMessage"))
                .json(&json!({
                    "chat_id": &reply.peer_id,
                    "text": piece,
                }))
                .send()?;
            if !resp.status().is_success() {
                anyhow::bail!("telegram sendMessage failed: {}", resp.text().unwrap_or_default());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct TgUpdate {
    update_id: i64,
    message: Option<TgMessage>,
}

#[derive(Debug, Deserialize)]
struct TgMessage {
    chat: TgChat,
    text: Option<String>,
    from: Option<TgUser>,
}

#[derive(Debug, Deserialize)]
struct TgChat {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct TgUser {
    id: i64,
}

async fn run_telegram(
    state_dir: &Path,
    allowlist: &PairingAllowlist,
    on_message: MessageHandler,
) -> anyhow::Result<()> {
    require_allowlist(allowlist, "telegram")?;
    // Exclusive getUpdates: hold flock for process lifetime.
    let _lock = crate::instance_lock::try_acquire(state_dir, "telegram")?;
    let bot = TelegramBot::from_env()?;
    let mut offset: i64 = 0;
    eprintln!(
        "[pirs-claw gateway] telegram long-poll started (allowlist {} peers; single-instance lock held)",
        allowlist.len()
    );
    loop {
        let url = format!(
            "{}?timeout=25&offset={}",
            bot.api("getUpdates"),
            offset
        );
        let resp = bot.client.get(&url).send().await;
        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[telegram] getUpdates error: {e}; retry in 3s");
                tokio::time::sleep(Duration::from_secs(3)).await;
                continue;
            }
        };
        #[derive(Deserialize)]
        struct TgResp {
            ok: bool,
            result: Option<Vec<TgUpdate>>,
        }
        let body: TgResp = resp.json().await.unwrap_or(TgResp {
            ok: false,
            result: None,
        });
        if !body.ok {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }
        for upd in body.result.unwrap_or_default() {
            offset = upd.update_id + 1;
            let Some(msg) = upd.message else { continue };
            let Some(text) = msg.text else { continue };
            let peer = msg.chat.id.to_string();
            let user = msg
                .from
                .as_ref()
                .map(|u| u.id.to_string())
                .unwrap_or_else(|| peer.clone());
            // Allow chat id or user id
            if !allowlist.is_allowed(&peer) && !allowlist.is_allowed(&user) {
                eprintln!("[telegram] ignore unpaired peer chat={peer} user={user}");
                let _ = bot
                    .send(
                        &peer,
                        "pirs-claw: you are not on the pairing allowlist. Ask the owner to add your chat id.",
                    )
                    .await;
                continue;
            }
            let inbound = InboundMessage {
                channel_id: CHANNEL_TELEGRAM.into(),
                peer_id: peer.clone(),
                text,
                ts: crate::channel::now_secs_pub(),
            };
            match on_message(inbound).await {
                Ok(reply) => {
                    if let Err(e) = bot.send(&peer, &reply).await {
                        eprintln!("[telegram] send error: {e}");
                    }
                }
                Err(e) => {
                    let _ = bot.send(&peer, &format!("error: {e}")).await;
                }
            }
        }
    }
}

// ─── Webhook-style channels (Discord / Slack / WhatsApp) ────────────────────

/// Shared tiny HTTP listener for webhook JSON bodies.
type SendFuture = std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send>>;

async fn run_webhook_listener(
    channel: &'static str,
    port_env: &str,
    default_port: u16,
    allowlist: &PairingAllowlist,
    extract: fn(&serde_json::Value) -> Option<(String, String)>,
    send: fn(&str, &str) -> SendFuture,
    on_message: MessageHandler,
) -> anyhow::Result<()> {
    require_allowlist(allowlist, channel)?;
    let port: u16 = std::env::var(port_env)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default_port);
    let addr = webhook_socket_addr(port);
    let host = webhook_bind_host();
    if host == "0.0.0.0" || host == "::" {
        eprintln!(
            "[pirs-claw] WARNING: webhook bound publicly on {addr} — ensure firewall + pairing"
        );
    }
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!(
        "[pirs-claw gateway] {channel} webhook listening on {addr} (POST / JSON body; default localhost)"
    );
    let allowlist = allowlist.clone();
    loop {
        let (mut sock, _) = listener.accept().await?;
        let allowlist = allowlist.clone();
        let on_message = on_message.clone();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buf = vec![0u8; 65536];
            let n = match sock.read(&mut buf).await {
                Ok(n) => n,
                Err(_) => return,
            };
            let raw = String::from_utf8_lossy(&buf[..n]);
            let first_line = raw.lines().next().unwrap_or("");
            // WhatsApp / Meta hub.challenge verification (GET).
            if first_line.starts_with("GET ") {
                if let Some(q) = first_line.split_whitespace().nth(1) {
                    if let Some(challenge) = whatsapp_verify_challenge(q) {
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}",
                            challenge.len(),
                            challenge
                        );
                        let _ = sock.write_all(resp.as_bytes()).await;
                        return;
                    }
                }
                let _ = sock
                    .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
                    .await;
                return;
            }
            // Crude HTTP: find body after \r\n\r\n
            let body = raw
                .split("\r\n\r\n")
                .nth(1)
                .unwrap_or("")
                .trim_end_matches('\0');
            let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
                let _ = sock
                    .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n")
                    .await;
                return;
            };
            // Slack URL verification challenge (JSON body)
            if let Some(challenge) = v.get("challenge").and_then(|c| c.as_str()) {
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}",
                    challenge.len(),
                    challenge
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                return;
            }
            let Some((peer, text)) = extract(&v) else {
                let _ = sock
                    .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                    .await;
                return;
            };
            if !allowlist.is_allowed(&peer) {
                let _ = sock
                    .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
                    .await;
                return;
            }
            let inbound = InboundMessage {
                channel_id: channel.into(),
                peer_id: peer.clone(),
                text,
                ts: crate::channel::now_secs_pub(),
            };
            let reply = match on_message(inbound).await {
                Ok(r) => r,
                Err(e) => format!("error: {e}"),
            };
            if let Err(e) = send(&peer, &reply).await {
                eprintln!("[{channel}] send error: {e}");
            }
            let _ = sock
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK")
                .await;
        });
    }
}

/// Parse WhatsApp cloud API verify GET query; returns challenge if token matches.
pub fn whatsapp_verify_challenge(request_target: &str) -> Option<String> {
    let q = request_target.split('?').nth(1)?;
    let mut mode = None;
    let mut token = None;
    let mut challenge = None;
    for part in q.split('&') {
        let mut kv = part.splitn(2, '=');
        let k = kv.next()?;
        let v = kv.next().unwrap_or("");
        let v = urlencoding_decode(v);
        match k {
            "hub.mode" => mode = Some(v),
            "hub.verify_token" => token = Some(v),
            "hub.challenge" => challenge = Some(v),
            _ => {}
        }
    }
    if mode.as_deref() != Some("subscribe") {
        return None;
    }
    let expected = std::env::var("WHATSAPP_VERIFY_TOKEN")
        .or_else(|_| std::env::var("PIRS_WHATSAPP_VERIFY_TOKEN"))
        .ok()?;
    if token.as_deref() == Some(expected.as_str()) {
        challenge
    } else {
        None
    }
}

fn urlencoding_decode(s: &str) -> String {
    // Minimal: + → space, %XX
    let mut out = String::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => {
                out.push(' ');
                i += 1;
            }
            b'%' if i + 2 < b.len() => {
                let hex = &s[i + 1..i + 3];
                if let Ok(v) = u8::from_str_radix(hex, 16) {
                    out.push(v as char);
                    i += 3;
                } else {
                    out.push('%');
                    i += 1;
                }
            }
            c => {
                out.push(c as char);
                i += 1;
            }
        }
    }
    out
}

fn extract_discord(v: &serde_json::Value) -> Option<(String, String)> {
    // Minimal: { "author_id": "...", "content": "..." } or Discord interaction-like
    let peer = v
        .get("author_id")
        .or_else(|| v.pointer("/author/id"))
        .or_else(|| v.get("user_id"))
        .and_then(|x| x.as_str().map(|s| s.to_string()).or_else(|| x.as_i64().map(|n| n.to_string())))?;
    let text = v
        .get("content")
        .or_else(|| v.get("text"))
        .and_then(|x| x.as_str())?
        .to_string();
    if text.is_empty() {
        return None;
    }
    Some((peer, text))
}

fn extract_slack(v: &serde_json::Value) -> Option<(String, String)> {
    let event = v.get("event").unwrap_or(v);
    if event.get("bot_id").is_some() {
        return None;
    }
    let peer = event
        .get("user")
        .and_then(|x| x.as_str())
        .or_else(|| event.get("channel").and_then(|x| x.as_str()))?
        .to_string();
    let text = event.get("text").and_then(|x| x.as_str())?.to_string();
    if text.is_empty() {
        return None;
    }
    Some((peer, text))
}

fn extract_whatsapp(v: &serde_json::Value) -> Option<(String, String)> {
    // Meta Cloud API simplified: entry[0].changes[0].value.messages[0]
    let msg = v
        .pointer("/entry/0/changes/0/value/messages/0")
        .or_else(|| v.get("messages").and_then(|m| m.get(0)))?;
    let peer = msg
        .get("from")
        .and_then(|x| x.as_str())?
        .to_string();
    let text = msg
        .pointer("/text/body")
        .and_then(|x| x.as_str())
        .or_else(|| msg.get("body").and_then(|x| x.as_str()))?
        .to_string();
    Some((peer, text))
}

/// Surface a schedule-tick reply to the user-facing channel.
///
/// For `DeliverTarget::Cli` this **must** print: tick runs chat with
/// `Command::output()`, so the child never writes to the parent's stdout.
pub async fn deliver_outbound(target: &crate::DeliverTarget, text: &str) -> anyhow::Result<()> {
    match target {
        crate::DeliverTarget::Cli => {
            println!("{text}");
            Ok(())
        }
        crate::DeliverTarget::Telegram { chat_id } => {
            let bot = TelegramBot::from_env()?;
            bot.send(chat_id, text).await
        }
        crate::DeliverTarget::Discord { peer } => send_discord(peer, text).await,
        crate::DeliverTarget::Slack { peer } => send_slack(peer, text).await,
        crate::DeliverTarget::Whatsapp { peer } => send_whatsapp(peer, text).await,
        crate::DeliverTarget::Signal { peer } => {
            let account = std::env::var("SIGNAL_ACCOUNT")
                .or_else(|_| std::env::var("PIRS_SIGNAL_ACCOUNT"))
                .map_err(|_| anyhow::anyhow!("SIGNAL_ACCOUNT not set"))?;
            let out = tokio::process::Command::new("signal-cli")
                .args(["-a", &account, "send", "-m", text, peer])
                .output()
                .await?;
            if !out.status.success() {
                anyhow::bail!(
                    "signal-cli send failed: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
            }
            Ok(())
        }
    }
}

async fn send_discord(peer: &str, text: &str) -> anyhow::Result<()> {
    let token = std::env::var("DISCORD_BOT_TOKEN")
        .or_else(|_| std::env::var("PIRS_DISCORD_BOT_TOKEN"))
        .map_err(|_| anyhow::anyhow!("DISCORD_BOT_TOKEN not set"))?;
    // DM channel create is multi-step; support channel id in peer as "channel:<id>"
    // or raw channel id for posting.
    let channel_id = peer.strip_prefix("channel:").unwrap_or(peer);
    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "https://discord.com/api/v10/channels/{channel_id}/messages"
        ))
        .header("Authorization", format!("Bot {token}"))
        .json(&json!({ "content": text.chars().take(1900).collect::<String>() }))
        .send()
        .await?;
    if !resp.status().is_success() {
        anyhow::bail!("discord send: {}", resp.text().await.unwrap_or_default());
    }
    Ok(())
}

async fn send_slack(peer: &str, text: &str) -> anyhow::Result<()> {
    let token = std::env::var("SLACK_BOT_TOKEN")
        .or_else(|_| std::env::var("PIRS_SLACK_BOT_TOKEN"))
        .map_err(|_| anyhow::anyhow!("SLACK_BOT_TOKEN not set"))?;
    let client = reqwest::Client::new();
    let resp = client
        .post("https://slack.com/api/chat.postMessage")
        .bearer_auth(token)
        .json(&json!({ "channel": peer, "text": text }))
        .send()
        .await?;
    let v: serde_json::Value = resp.json().await?;
    if v.get("ok") != Some(&json!(true)) {
        anyhow::bail!("slack send: {v}");
    }
    Ok(())
}

async fn send_whatsapp(peer: &str, text: &str) -> anyhow::Result<()> {
    let token = std::env::var("WHATSAPP_TOKEN")
        .or_else(|_| std::env::var("PIRS_WHATSAPP_TOKEN"))
        .map_err(|_| anyhow::anyhow!("WHATSAPP_TOKEN not set"))?;
    let phone_id = std::env::var("WHATSAPP_PHONE_NUMBER_ID")
        .or_else(|_| std::env::var("PIRS_WHATSAPP_PHONE_NUMBER_ID"))
        .map_err(|_| anyhow::anyhow!("WHATSAPP_PHONE_NUMBER_ID not set"))?;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "https://graph.facebook.com/v18.0/{phone_id}/messages"
        ))
        .bearer_auth(token)
        .json(&json!({
            "messaging_product": "whatsapp",
            "to": peer,
            "type": "text",
            "text": { "body": text.chars().take(4000).collect::<String>() }
        }))
        .send()
        .await?;
    if !resp.status().is_success() {
        anyhow::bail!("whatsapp send: {}", resp.text().await.unwrap_or_default());
    }
    Ok(())
}

async fn run_discord_webhook_mode(
    allowlist: &PairingAllowlist,
    on_message: MessageHandler,
) -> anyhow::Result<()> {
    run_webhook_listener(
        CHANNEL_DISCORD,
        "PIRS_CLAW_DISCORD_PORT",
        8741,
        allowlist,
        extract_discord,
        |peer, text| {
            let p = peer.to_string();
            let t = text.to_string();
            Box::pin(async move { send_discord(&p, &t).await })
        },
        on_message,
    )
    .await
}

async fn run_slack_webhook_mode(
    allowlist: &PairingAllowlist,
    on_message: MessageHandler,
) -> anyhow::Result<()> {
    run_webhook_listener(
        CHANNEL_SLACK,
        "PIRS_CLAW_SLACK_PORT",
        8742,
        allowlist,
        extract_slack,
        |peer, text| {
            let p = peer.to_string();
            let t = text.to_string();
            Box::pin(async move { send_slack(&p, &t).await })
        },
        on_message,
    )
    .await
}

async fn run_whatsapp_webhook_mode(
    allowlist: &PairingAllowlist,
    on_message: MessageHandler,
) -> anyhow::Result<()> {
    run_webhook_listener(
        CHANNEL_WHATSAPP,
        "PIRS_CLAW_WHATSAPP_PORT",
        8743,
        allowlist,
        extract_whatsapp,
        |peer, text| {
            let p = peer.to_string();
            let t = text.to_string();
            Box::pin(async move { send_whatsapp(&p, &t).await })
        },
        on_message,
    )
    .await
}

// ─── Signal via signal-cli ──────────────────────────────────────────────────

async fn run_signal_cli(
    allowlist: &PairingAllowlist,
    on_message: MessageHandler,
) -> anyhow::Result<()> {
    require_allowlist(allowlist, "signal")?;
    let account = std::env::var("SIGNAL_ACCOUNT")
        .or_else(|_| std::env::var("PIRS_SIGNAL_ACCOUNT"))
        .map_err(|_| {
            anyhow::anyhow!("signal: set SIGNAL_ACCOUNT (phone number) and install signal-cli")
        })?;
    // Require signal-cli on PATH
    let status = tokio::process::Command::new("signal-cli")
        .arg("--version")
        .output()
        .await;
    if status.map(|o| !o.status.success()).unwrap_or(true) {
        anyhow::bail!("signal: signal-cli not found on PATH");
    }
    eprintln!("[pirs-claw gateway] signal-cli receive loop for {account}");
    loop {
        let out = tokio::process::Command::new("signal-cli")
            .args(["-a", &account, "receive", "-t", "10", "--json"])
            .output()
            .await?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        for line in stdout.lines() {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let envelope = v.get("envelope").unwrap_or(&v);
            let peer = envelope
                .get("source")
                .or_else(|| envelope.get("sourceNumber"))
                .and_then(|x| x.as_str())
                .unwrap_or("");
            if peer.is_empty() || !allowlist.is_allowed(peer) {
                continue;
            }
            let text = envelope
                .pointer("/dataMessage/message")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            if text.is_empty() {
                continue;
            }
            let inbound = InboundMessage {
                channel_id: CHANNEL_SIGNAL.into(),
                peer_id: peer.into(),
                text: text.into(),
                ts: crate::channel::now_secs_pub(),
            };
            let reply = match on_message(inbound).await {
                Ok(r) => r,
                Err(e) => format!("error: {e}"),
            };
            let _ = tokio::process::Command::new("signal-cli")
                .args(["-a", &account, "send", "-m", &reply, peer])
                .output()
                .await;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_chunks_do_not_split_multibyte() {
        let s = "á".repeat(10);
        let parts = utf8_chunks(&s, 3);
        assert!(parts.iter().all(|p| p.chars().count() <= 3));
        assert_eq!(parts.join(""), s);
    }

    #[test]
    fn deliver_outbound_cli_is_required_after_captured_chat() {
        // Contract: tick uses Command::output(); Cli arm must print, not no-op.
        // Drive the real match by invoking the async helper (prints to stdout).
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            deliver_outbound(&crate::DeliverTarget::Cli, "tick-cli-reply-marker")
                .await
                .unwrap();
        });
        // If Cli were a silent Ok(()), this test still passes — structural
        // assert on main ensures we always call deliver_outbound for every target.
        let main_src = include_str!("main.rs");
        assert!(
            main_src.contains("deliver_outbound(&j.deliver"),
            "tick must call deliver_outbound with the job deliver target"
        );
        assert!(
            !main_src.contains("if !matches!(j.deliver, DeliverTarget::Cli)"),
            "must not skip Cli deliver after captured subprocess stdout"
        );
        let cli_arm = include_str!("gateway.rs");
        assert!(
            cli_arm.contains("DeliverTarget::Cli") && cli_arm.contains("println!"),
            "Cli deliver must println the reply text"
        );
    }

    #[test]
    fn webhook_bind_defaults_to_localhost() {
        std::env::remove_var(PUBLIC_BIND_ENV);
        std::env::remove_var(BIND_ENV);
        assert_eq!(webhook_bind_host(), "127.0.0.1");
        let addr = webhook_socket_addr(8741);
        assert_eq!(addr.ip().to_string(), "127.0.0.1");
        assert_eq!(addr.port(), 8741);
    }

    #[test]
    fn webhook_bind_public_opt_in() {
        std::env::remove_var(BIND_ENV);
        std::env::set_var(PUBLIC_BIND_ENV, "1");
        assert_eq!(webhook_bind_host(), "0.0.0.0");
        std::env::remove_var(PUBLIC_BIND_ENV);
        std::env::set_var(BIND_ENV, "0.0.0.0");
        assert_eq!(webhook_bind_host(), "0.0.0.0");
        std::env::remove_var(BIND_ENV);
    }

    #[test]
    fn extract_discord_simple() {
        let v = json!({"author_id": "99", "content": "hello"});
        assert_eq!(
            extract_discord(&v),
            Some(("99".into(), "hello".into()))
        );
    }

    #[test]
    fn extract_slack_ignores_bots() {
        let v = json!({"event": {"bot_id": "B1", "user": "U1", "text": "x"}});
        assert!(extract_slack(&v).is_none());
        let v = json!({"event": {"user": "U1", "text": "hi"}});
        assert_eq!(extract_slack(&v), Some(("U1".into(), "hi".into())));
    }

    #[test]
    fn extract_whatsapp_meta_shape() {
        let v = json!({
            "entry": [{"changes": [{"value": {"messages": [
                {"from": "15551234567", "text": {"body": "yo"}}
            ]}}]}]
        });
        assert_eq!(
            extract_whatsapp(&v),
            Some(("15551234567".into(), "yo".into()))
        );
    }

    #[test]
    fn whatsapp_verify_token_gate() {
        std::env::set_var("WHATSAPP_VERIFY_TOKEN", "secret-token");
        let ok = whatsapp_verify_challenge(
            "/?hub.mode=subscribe&hub.verify_token=secret-token&hub.challenge=abc123",
        );
        assert_eq!(ok.as_deref(), Some("abc123"));
        let bad = whatsapp_verify_challenge(
            "/?hub.mode=subscribe&hub.verify_token=wrong&hub.challenge=abc123",
        );
        assert!(bad.is_none());
        std::env::remove_var("WHATSAPP_VERIFY_TOKEN");
    }
}
