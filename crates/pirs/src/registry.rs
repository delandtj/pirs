//! Named backends + model aliases for multi-subscription routing.
//!
//! Loaded from `[[backends]]` / `[[models]]` in user and project `config.toml`.
//! Project-layer **backends** are allowed only when the project directory is
//! trusted (`pirs trust`); otherwise they are stripped with a warning. Model
//! aliases may live in either layer.
//!
//! `serve` is an ordered list — first entry is primary; later entries are
//! failover when the primary stream errors before producing content.
//!
//! ```toml
//! [[backends]]
//! name = "openrouter"
//! kind = "openai_compatible"
//! base_url = "https://openrouter.ai/api/v1"
//! api_key_env = "OPENROUTER_API_KEY"
//! headers = { HTTP-Referer = "https://example.com", X-Title = "pirs" }
//!
//! [[models]]
//! alias = "deepseek-v4-flash"
//! serve = [
//!   { backend = "openrouter", model = "deepseek/deepseek-v4-flash" },
//!   { backend = "dashscope", model = "deepseek-v4-flash" },  # failover
//! ]
//! ```

use std::collections::HashMap;
use std::path::Path;
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
    #[allow(dead_code)]
    pub persona: Option<String>,
    pub tier: Option<String>,
    pub ctx: Option<u64>,
    #[serde(default)]
    #[allow(dead_code)]
    pub caps: Vec<String>,
    /// Ordered serve targets; first is primary, rest are failover.
    pub serve: Vec<ServeEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServeEntry {
    pub backend: String,
    pub model: String,
}

pub fn merge(base: RegistryFile, over: RegistryFile) -> RegistryFile {
    let mut backends = base.backends;
    for b in over.backends {
        if let Some(i) = backends.iter().position(|x| x.name == b.name) {
            backends[i] = b;
        } else {
            backends.push(b);
        }
    }
    let mut models = base.models;
    for m in over.models {
        if let Some(i) = models.iter().position(|x| x.alias == m.alias) {
            models[i] = m;
        } else {
            models.push(m);
        }
    }
    RegistryFile { backends, models }
}

#[cfg(test)]
pub fn parse_registry_toml(text: &str) -> anyhow::Result<RegistryFile> {
    Ok(toml::from_str(text)?)
}

pub fn parse_from_config_value(value: &toml::Value) -> RegistryFile {
    let table = match value.as_table() {
        Some(t) => t,
        None => return RegistryFile::default(),
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
            eprintln!("[warning: backends/models in config.toml ignored: {e}]");
            RegistryFile::default()
        }
    }
}

/// True when the project's `.pirs/extensions` is in the pirs trust store.
/// Trust keys look like `{canonical_extensions_path}#{scripts_hash}` (see
/// `pirs_rhai::trust_directory`).
fn project_is_trusted(project_pirs_dir: &Path) -> bool {
    let Ok(home) = std::env::var("HOME") else {
        return false;
    };
    let path = Path::new(&home).join(".pirs").join("trusted.json");
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let set: std::collections::HashSet<String> = serde_json::from_str(&text).unwrap_or_default();
    if set.is_empty() {
        return false;
    }
    let ext = project_pirs_dir.join("extensions");
    let ext = ext.canonicalize().unwrap_or(ext);
    let prefix = format!("{}#", ext.display());
    let path_s = ext.to_string_lossy();
    set.iter()
        .any(|t| t.starts_with(&prefix) || t == path_s.as_ref() || t.starts_with(path_s.as_ref()))
}

pub fn load_registry_layers(cwd: &Path) -> RegistryFile {
    let mut reg = RegistryFile::default();
    if let Some(path) = crate::config_file::user_config_path() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(v) = text.parse::<toml::Value>() {
                reg = merge(reg, parse_from_config_value(&v));
            }
        }
    }
    if let Some(path) = crate::config_file::find_project_config(cwd) {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(v) = text.parse::<toml::Value>() {
                let mut project = parse_from_config_value(&v);
                if !project.backends.is_empty() {
                    let project_dir = path.parent().unwrap_or(Path::new("."));
                    if project_is_trusted(project_dir) {
                        eprintln!(
                            "[model registry: loading {} backend(s) from trusted project config {}]",
                            project.backends.len(),
                            path.display()
                        );
                    } else {
                        eprintln!(
                            "[note: project {} defines backends but is not trusted — \
                             backends ignored (run `pirs trust` in the project, or move \
                             backends to ~/.pirs/config.toml). Model aliases still load.]",
                            path.display()
                        );
                        project.backends.clear();
                    }
                }
                reg = merge(reg, project);
            }
        }
    }
    reg
}

pub fn build_routing_provider(
    registry: &RegistryFile,
    default: Arc<dyn LlmProvider>,
    default_api_key: Option<String>,
    max_retries: u32,
) -> anyhow::Result<Option<Arc<RoutingProvider>>> {
    if registry.backends.is_empty() && registry.models.is_empty() {
        return Ok(None);
    }
    if registry.models.is_empty() {
        return Ok(None);
    }

    let mut backend_handles: HashMap<
        String,
        (
            Arc<dyn LlmProvider>,
            Option<String>,
            Vec<(String, String)>,
        ),
    > = HashMap::new();

    for b in &registry.backends {
        let kind = BackendKind::parse(&b.kind)
            .ok_or_else(|| anyhow!("backend {:?}: unknown kind {:?}", b.name, b.kind))?;
        let api_key = b
            .api_key_env
            .as_ref()
            .and_then(|env| std::env::var(env).ok())
            .filter(|s| !s.is_empty());
        if b.api_key_env.is_some() && api_key.is_none() {
            eprintln!(
                "[warning: backend {:?} api_key_env {:?} is unset]",
                b.name,
                b.api_key_env.as_deref().unwrap_or("")
            );
        }
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
                    "model alias {:?} serves backend {:?} which is not defined in [[backends]]",
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

    for r in &routes {
        for s in &r.serve {
            if !backend_handles.contains_key(&s.backend) {
                bail!(
                    "model alias {:?} needs backend {:?} — add it under [[backends]]",
                    r.alias,
                    s.backend
                );
            }
        }
    }

    Ok(Some(Arc::new(RoutingProvider::new(
        default,
        default_api_key,
        vec![],
        backend_handles,
        routes,
    ))))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[[backends]]
name = "openrouter"
kind = "openai_compatible"
base_url = "https://openrouter.ai/api/v1"
api_key_env = "OPENROUTER_API_KEY"
headers = { HTTP-Referer = "https://example.com", X-Title = "pirs" }

[[backends]]
name = "dashscope"
kind = "openai_compatible"
base_url = "https://coding-intl.dashscope.aliyuncs.com/v1"
api_key_env = "DASHSCOPE_API_KEY"

[[models]]
alias = "deepseek-v4-flash"
tier = "fast"
ctx = 1000000
serve = [
  { backend = "openrouter", model = "deepseek/deepseek-v4-flash" },
  { backend = "dashscope", model = "deepseek-v4-flash" },
]

[[models]]
alias = "qwen-plus"
serve = [{ backend = "dashscope", model = "qwen3.5-plus" }]
"#;

    #[test]
    fn parses_backends_and_failover_serve_list() {
        let reg = parse_registry_toml(SAMPLE).unwrap();
        assert_eq!(reg.backends.len(), 2);
        let m = reg
            .models
            .iter()
            .find(|m| m.alias == "deepseek-v4-flash")
            .unwrap();
        assert_eq!(m.serve.len(), 2);
        assert_eq!(m.serve[0].backend, "openrouter");
        assert_eq!(m.serve[1].backend, "dashscope");
    }

    #[test]
    fn parse_from_full_config_ignores_other_keys() {
        let text = r#"
model = "qwen-plus"
provider = "openai"

[[backends]]
name = "dashscope"
kind = "openai_compatible"
base_url = "https://example.com/v1"
api_key_env = "DASHSCOPE_API_KEY"

[[models]]
alias = "qwen-plus"
serve = [{ backend = "dashscope", model = "qwen3.5-plus" }]
"#;
        let v: toml::Value = text.parse().unwrap();
        let reg = parse_from_config_value(&v);
        assert_eq!(reg.backends.len(), 1);
        assert_eq!(reg.models[0].alias, "qwen-plus");
    }

    #[test]
    fn merge_prefers_overlay_alias() {
        let base = parse_registry_toml(
            r#"
[[models]]
alias = "x"
serve = [{ backend = "a", model = "old" }]
"#,
        )
        .unwrap();
        let over = parse_registry_toml(
            r#"
[[models]]
alias = "x"
serve = [{ backend = "b", model = "new" }]
"#,
        )
        .unwrap();
        let m = merge(base, over);
        assert_eq!(m.models.len(), 1);
        assert_eq!(m.models[0].serve[0].model, "new");
    }

    #[test]
    fn build_routing_keeps_full_serve_chain() {
        let reg = parse_registry_toml(SAMPLE).unwrap();
        // Don't need real keys for construction — empty env is fine.
        let default: Arc<dyn LlmProvider> = Arc::new(OpenAiCompat::new(None));
        let router = build_routing_provider(&reg, default, None, 0)
            .unwrap()
            .expect("router");
        let targets = router.targets_for("deepseek-v4-flash");
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].backend_name, "openrouter");
        assert_eq!(targets[1].backend_name, "dashscope");
    }
}
