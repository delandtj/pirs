//! Multi-backend model registry for claw (same `~/.pirs/config.toml` shape as `pirs`).
//!
//! Loads user-level `[[backends]]` / `[[models]]` only (project backends stay on
//! the `pirs` harness with trust). Falls back to env OpenAI-compat when empty.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, bail};
use pirs_ai::{
    AnthropicClient, BackendKind, LlmProvider, ModelRoute, OpenAiCompat, RoutingProvider,
    ServeTarget,
};
use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RegistryFile {
    #[serde(default)]
    pub backends: Vec<BackendEntry>,
    #[serde(default)]
    pub models: Vec<ModelEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BackendEntry {
    pub name: String,
    #[serde(default = "default_kind")]
    pub kind: String,
    pub base_url: String,
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

fn default_kind() -> String {
    "openai_compatible".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelEntry {
    pub alias: String,
    pub tier: Option<String>,
    pub ctx: Option<u64>,
    #[serde(default)]
    pub serve: Vec<ServeEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServeEntry {
    pub backend: String,
    pub model: String,
}

pub fn user_config_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".pirs").join("config.toml"))
}

pub fn load_user_registry() -> RegistryFile {
    let Some(path) = user_config_path() else {
        return RegistryFile::default();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return RegistryFile::default();
    };
    let Ok(v) = text.parse::<toml::Value>() else {
        return RegistryFile::default();
    };
    parse_from_config_value(&v)
}

fn parse_from_config_value(value: &toml::Value) -> RegistryFile {
    let Some(table) = value.as_table() else {
        return RegistryFile::default();
    };
    let mut partial = toml::map::Map::new();
    if let Some(b) = table.get("backends") {
        partial.insert("backends".into(), b.clone());
    }
    if let Some(m) = table.get("models") {
        partial.insert("models".into(), m.clone());
    }
    if partial.is_empty() {
        return RegistryFile::default();
    }
    match toml::Value::Table(partial).try_into::<RegistryFile>() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[pirs-claw registry: parse warning: {e}]");
            RegistryFile::default()
        }
    }
}

pub fn api_key_for_alias(registry: &RegistryFile, alias: &str) -> Option<String> {
    let model = registry.models.iter().find(|m| m.alias == alias)?;
    let serve = model.serve.first()?;
    let backend = registry.backends.iter().find(|b| b.name == serve.backend)?;
    let env = backend.api_key_env.as_ref()?;
    std::env::var(env).ok().filter(|s| !s.is_empty())
}

pub fn first_available_backend_key(registry: &RegistryFile) -> Option<String> {
    for b in &registry.backends {
        if let Some(env) = &b.api_key_env {
            if let Ok(k) = std::env::var(env) {
                if !k.is_empty() {
                    return Some(k);
                }
            }
        }
    }
    None
}

pub fn build_routing_provider(
    registry: &RegistryFile,
    default: Arc<dyn LlmProvider>,
    default_api_key: Option<String>,
    max_retries: u32,
) -> anyhow::Result<Option<Arc<RoutingProvider>>> {
    if registry.models.is_empty() {
        return Ok(None);
    }
    type BackendHandle = (Arc<dyn LlmProvider>, Option<String>, Vec<(String, String)>);
    let mut backend_handles: HashMap<String, BackendHandle> = HashMap::new();

    for b in &registry.backends {
        let kind = BackendKind::parse(&b.kind)
            .ok_or_else(|| anyhow!("backend {:?}: unknown kind {:?}", b.name, b.kind))?;
        let api_key = b
            .api_key_env
            .as_ref()
            .and_then(|env| std::env::var(env).ok())
            .filter(|s| !s.is_empty());
        let headers: Vec<(String, String)> =
            b.headers.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        let provider: Arc<dyn LlmProvider> = match kind {
            BackendKind::OpenaiCompatible => Arc::new(
                OpenAiCompat::new(Some(b.base_url.clone()))
                    .with_max_retries(max_retries)
                    .with_provider_name(b.name.clone()),
            ),
            BackendKind::Anthropic => Arc::new(
                AnthropicClient::new(Some(b.base_url.clone())).with_max_retries(max_retries),
            ),
        };
        backend_handles.insert(b.name.clone(), (provider, api_key, headers));
    }

    let mut routes = Vec::new();
    for m in &registry.models {
        if m.serve.is_empty() {
            bail!("model alias {:?} has empty serve list", m.alias);
        }
        let mut serve = Vec::new();
        for s in &m.serve {
            if !backend_handles.contains_key(&s.backend) && s.backend != "default" {
                bail!(
                    "model alias {:?} serves unknown backend {:?}",
                    m.alias,
                    s.backend
                );
            }
            serve.push(ServeTarget {
                backend: s.backend.clone(),
                remote_model: s.model.clone(),
            });
        }
        routes.push(ModelRoute {
            alias: m.alias.clone(),
            serve,
            tier: m.tier.clone(),
            ctx: m.ctx,
        });
    }

    if routes
        .iter()
        .any(|r| r.serve.iter().any(|s| s.backend == "default"))
    {
        backend_handles.insert(
            "default".into(),
            (Arc::clone(&default), default_api_key.clone(), vec![]),
        );
    }

    Ok(Some(Arc::new(RoutingProvider::new(
        default,
        default_api_key,
        vec![],
        backend_handles,
        routes,
    ))))
}

/// Resolve provider for a model name or registry alias.
pub fn resolve_llm(
    model: &str,
    max_retries: u32,
) -> anyhow::Result<(Arc<dyn LlmProvider>, Option<String>, bool)> {
    use crate::secrets::resolve_provider_and_key;

    let (base, env_key) = resolve_provider_and_key();
    let default: Arc<dyn LlmProvider> =
        Arc::new(OpenAiCompat::new(base).with_max_retries(max_retries));

    let reg = load_user_registry();
    if !reg.models.is_empty() {
        if let Some(router) = build_routing_provider(&reg, Arc::clone(&default), env_key.clone(), max_retries)?
        {
            if router.has_alias(model) {
                let key = api_key_for_alias(&reg, model)
                    .or_else(|| first_available_backend_key(&reg))
                    .or(env_key);
                eprintln!(
                    "[pirs-claw registry: alias {model:?} via ~/.pirs/config.toml ({} model(s))]",
                    reg.models.len()
                );
                return Ok((router, key, true));
            }
        }
    }
    Ok((default, env_key, false))
}

/// Whether a path looks like a config.toml with backends (tests).
pub fn registry_file_has_models(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(v) = text.parse::<toml::Value>() else {
        return false;
    };
    !parse_from_config_value(&v).models.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sample_registry() {
        let sample = r#"
[[backends]]
name = "dashscope"
kind = "openai_compatible"
base_url = "https://example.com/v1"
api_key_env = "DASHSCOPE_API_KEY"

[[models]]
alias = "qwen3.5-plus"
serve = [{ backend = "dashscope", model = "qwen3.5-plus" }]
"#;
        let v: toml::Value = sample.parse().unwrap();
        let reg = parse_from_config_value(&v);
        assert_eq!(reg.backends.len(), 1);
        assert_eq!(reg.models.len(), 1);
        assert_eq!(reg.models[0].alias, "qwen3.5-plus");
    }
}
