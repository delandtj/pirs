//! Skills loader (Hermes / agentskills.io-shaped).
//!
//! Loads `~/.pirs/skills/**/SKILL.md` (or `*.md`) into a system-prompt appendix.
//! Compatible with skill-crystallizer.rhai output frontmatter.

use std::fs;
use std::path::{Path, PathBuf};

/// Default skills root: `~/.pirs/skills`.
pub fn default_skills_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".pirs").join("skills")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub body: String,
    pub path: PathBuf,
}

/// Parse optional YAML frontmatter between `---` fences.
pub fn parse_skill_md(raw: &str, path: &Path) -> Skill {
    let mut name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("skill")
        .to_string();
    let mut description = String::new();
    let body;

    if let Some(rest) = raw.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            let fm = &rest[..end];
            body = rest[end + 4..].trim_start().to_string();
            for line in fm.lines() {
                let line = line.trim();
                if let Some(v) = line.strip_prefix("name:") {
                    name = v.trim().trim_matches('"').to_string();
                } else if let Some(v) = line.strip_prefix("description:") {
                    description = v.trim().trim_matches('"').to_string();
                }
            }
        } else {
            body = raw.to_string();
        }
    } else {
        body = raw.to_string();
    }

    Skill {
        name,
        description,
        body: body.trim().to_string(),
        path: path.to_path_buf(),
    }
}

/// Walk skills dir for SKILL.md or any .md (max depth 3).
pub fn load_skills(dir: &Path) -> Vec<Skill> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        return out;
    }
    walk(dir, 0, &mut out);
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn walk(dir: &Path, depth: u32, out: &mut Vec<Skill>) {
    if depth > 3 {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for ent in rd.flatten() {
        let p = ent.path();
        if p.is_dir() {
            // Prefer SKILL.md inside skill folders
            let skill_md = p.join("SKILL.md");
            if skill_md.is_file() {
                if let Ok(raw) = fs::read_to_string(&skill_md) {
                    out.push(parse_skill_md(&raw, &skill_md));
                }
            } else {
                walk(&p, depth + 1, out);
            }
        } else if p.extension().and_then(|e| e.to_str()) == Some("md") {
            if let Ok(raw) = fs::read_to_string(&p) {
                out.push(parse_skill_md(&raw, &p));
            }
        }
    }
}

/// Format skills for injection into a system prompt.
pub fn skills_prompt_section(skills: &[Skill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut s = String::from("\n\n## Available skills\n");
    for sk in skills {
        s.push_str(&format!(
            "\n### {}\n{}\n\n{}\n",
            sk.name,
            if sk.description.is_empty() {
                ""
            } else {
                sk.description.as_str()
            },
            sk.body
        ));
        record_usage(&sk.name);
    }
    s
}

/// Find a skill by name (case-sensitive).
pub fn find_skill<'a>(skills: &'a [Skill], name: &str) -> Option<&'a Skill> {
    skills.iter().find(|s| s.name == name)
}

/// Copy a skill file or directory into `~/.pirs/skills/<name>/SKILL.md`.
pub fn install_skill(src: &Path, dest_root: &Path) -> anyhow::Result<PathBuf> {
    if !src.exists() {
        anyhow::bail!("skill path not found: {}", src.display());
    }
    fs::create_dir_all(dest_root)?;
    let (name, content) = if src.is_dir() {
        let skill_md = src.join("SKILL.md");
        let raw = fs::read_to_string(&skill_md)
            .map_err(|e| anyhow::anyhow!("read {}: {e}", skill_md.display()))?;
        let sk = parse_skill_md(&raw, &skill_md);
        (sk.name, raw)
    } else {
        let raw = fs::read_to_string(src)?;
        let sk = parse_skill_md(&raw, src);
        (sk.name, raw)
    };
    let dir = dest_root.join(&name);
    fs::create_dir_all(&dir)?;
    let dest = dir.join("SKILL.md");
    fs::write(&dest, content)?;
    Ok(dest)
}

fn usage_path() -> PathBuf {
    default_skills_dir().join("usage.json")
}

/// Bump usage counter for a skill name (best-effort).
pub fn record_usage(name: &str) {
    let path = usage_path();
    let mut map: std::collections::BTreeMap<String, u64> = fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    *map.entry(name.to_string()).or_insert(0) += 1;
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, serde_json::to_string_pretty(&map).unwrap_or_default());
}

pub fn usage_counts() -> std::collections::BTreeMap<String, u64> {
    fs::read_to_string(usage_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_frontmatter_skill() {
        let raw = "---\nname: fix-rust\ndescription: when cargo fails\n---\nRun cargo test.\n";
        let sk = parse_skill_md(raw, Path::new("/x/SKILL.md"));
        assert_eq!(sk.name, "fix-rust");
        assert_eq!(sk.description, "when cargo fails");
        assert!(sk.body.contains("cargo test"));
    }

    #[test]
    fn load_from_dir() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("my-skill");
        fs::create_dir_all(&nested).unwrap();
        fs::write(
            nested.join("SKILL.md"),
            "---\nname: my-skill\ndescription: demo\n---\nDo the thing.\n",
        )
        .unwrap();
        let skills = load_skills(dir.path());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "my-skill");
        let section = skills_prompt_section(&skills);
        assert!(section.contains("my-skill"));
        assert!(section.contains("Do the thing"));
    }

    #[test]
    fn install_skill_from_file() {
        let src_dir = tempfile::tempdir().unwrap();
        let dest_dir = tempfile::tempdir().unwrap();
        let src = src_dir.path().join("x.md");
        fs::write(
            &src,
            "---\nname: installed-skill\ndescription: d\n---\nBody here.\n",
        )
        .unwrap();
        let dest = install_skill(&src, dest_dir.path()).unwrap();
        assert!(dest.is_file());
        let loaded = load_skills(dest_dir.path());
        assert!(loaded.iter().any(|s| s.name == "installed-skill"));
    }
}
