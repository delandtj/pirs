//! Load `~/.pirs/secrets.env` (does not override existing env vars).

use std::path::PathBuf;

pub fn load_secrets_env() {
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let path = PathBuf::from(home).join(".pirs").join("secrets.env");
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let body = line.strip_prefix("export ").unwrap_or(line);
        let Some((k, v)) = body.split_once('=') else {
            continue;
        };
        let k = k.trim();
        if std::env::var_os(k).is_some() {
            continue;
        }
        let mut v = v.trim().to_string();
        if (v.starts_with('\'') && v.ends_with('\'')) || (v.starts_with('"') && v.ends_with('"')) {
            v = v[1..v.len() - 1].to_string();
        }
        if v.starts_with("${") && v.ends_with('}') {
            let refn = &v[2..v.len() - 1];
            v = std::env::var(refn).unwrap_or_default();
            if v.is_empty() {
                continue;
            }
        }
        // SAFETY: process startup / before concurrent workers use these keys.
        std::env::set_var(k, v);
    }
}

/// Resolve OpenAI-compatible base URL + API key from env (post-secrets load).
pub fn resolve_provider_and_key() -> (Option<String>, Option<String>) {
    if let Ok(base) = std::env::var("OPENAI_BASE_URL") {
        let key = std::env::var("OPENAI_API_KEY")
            .ok()
            .or_else(|| std::env::var("DASHSCOPE_API_KEY").ok())
            .or_else(|| std::env::var("DEEPSEEK_API_KEY").ok())
            .or_else(|| std::env::var("OPENROUTER_API_KEY").ok());
        return (Some(base), key);
    }
    if std::env::var("DASHSCOPE_API_KEY").is_ok() {
        return (
            Some("https://coding-intl.dashscope.aliyuncs.com/v1".into()),
            std::env::var("DASHSCOPE_API_KEY").ok(),
        );
    }
    if std::env::var("DEEPSEEK_API_KEY").is_ok() {
        return (
            Some("https://api.deepseek.com/v1".into()),
            std::env::var("DEEPSEEK_API_KEY").ok(),
        );
    }
    if std::env::var("OPENROUTER_API_KEY").is_ok() {
        return (
            Some("https://openrouter.ai/api/v1".into()),
            std::env::var("OPENROUTER_API_KEY").ok(),
        );
    }
    (None, std::env::var("OPENAI_API_KEY").ok())
}
