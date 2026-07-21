//! Wire pirs-agent FTS5 memory into claw sessions (Hermes memory gap).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pirs_agent::memory::MemoryStore;

/// Open claw memory DB under state dir.
pub fn open_memory(state_dir: &Path) -> anyhow::Result<Arc<MemoryStore>> {
    let path = state_dir.join("memory.db");
    MemoryStore::open(&path).map_err(|e| anyhow::anyhow!("memory open: {e}"))
}

/// Scope memory rows to a session key (channel/peer).
pub fn scope_session(store: &MemoryStore, session_key: &str) {
    store.set_session(session_key);
}

/// Persist a chat turn for later recall.
pub fn remember_turn(store: &MemoryStore, role: &str, text: &str) {
    store.add(role, "chat", text);
}

/// Keyword recall snippet for system prompt (top hits).
pub fn recall_context(store: &MemoryStore, query: &str, limit: usize) -> String {
    let hits = store.search(query, limit);
    if hits.is_empty() {
        return String::new();
    }
    let mut s = String::from("\n\n## Memory recall\n");
    for h in hits {
        s.push_str(&format!("- [{}] {}: {}\n", h.kind, h.name, h.snippet));
    }
    s
}

pub fn memory_db_path(state_dir: &Path) -> PathBuf {
    state_dir.join("memory.db")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remember_and_recall() {
        let dir = tempfile::tempdir().unwrap();
        let store = open_memory(dir.path()).unwrap();
        scope_session(&store, "cli/local");
        remember_turn(&store, "user", "my dog is named Pixel");
        remember_turn(&store, "assistant", "Got it about Pixel");
        let ctx = recall_context(&store, "Pixel", 5);
        assert!(ctx.contains("Pixel"), "{ctx}");
    }
}
