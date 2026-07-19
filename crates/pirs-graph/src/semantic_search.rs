//! `semantic_search` tool: natural-language retrieval over the code graph.
//!
//! The one retrieval mode grep and the structural graph don't cover. A query is
//! embedded via [`EmbeddingClient`], scored by cosine against per-symbol vectors
//! in the [`GraphStore`], then **reranked with graph centrality** so the results
//! are semantically-close AND structurally-important — "similar" alone is a poor
//! proxy for "relevant", so the graph gets a vote.
//!
//! The embedding index is built lazily on first search (embed everything not yet
//! embedded) and kept incrementally fresh thereafter. If the embedding service
//! is unreachable the tool degrades to a helpful note pointing at grep/code_map
//! rather than failing the run — semantic search is an aid, never a hard dep.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use pirs_agent::{AgentTool, ToolExecContext, ToolOutput};
use pirs_ai::EmbeddingClient;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::store::{GraphStore, SemanticHit};

/// Default source chars of a symbol to embed. Large enough for big-context
/// models (e.g. nomic-embed-text, 2048 tokens); small-context models (e.g.
/// all-minilm, 256 tokens) are handled by the per-item truncating fallback, so
/// no model-specific tuning is required. Override with --embed-max-chars.
const DEFAULT_MAX_CHUNK_CHARS: usize = 2000;
/// Batch size for embedding requests.
const EMBED_BATCH: usize = 64;

#[derive(Deserialize, JsonSchema)]
struct SemanticArgs {
    /// Natural-language description of what you're looking for
    /// (e.g. "where do we refresh the auth token").
    query: String,
    /// Max results (default 8).
    limit: Option<usize>,
}

pub struct SemanticSearchTool {
    graph: Arc<crate::LazyGraph>,
    root: PathBuf,
    db_path: PathBuf,
    embedder: EmbeddingClient,
    max_chars: usize,
}

impl SemanticSearchTool {
    pub fn new(
        graph: Arc<crate::LazyGraph>,
        root: PathBuf,
        db_path: PathBuf,
        embedder: EmbeddingClient,
        max_chars: Option<usize>,
    ) -> Self {
        SemanticSearchTool {
            graph,
            root,
            db_path,
            embedder,
            max_chars: max_chars.unwrap_or(DEFAULT_MAX_CHUNK_CHARS),
        }
    }

    /// Bring the embedding index up to date for the current model: stamp the
    /// model (wiping vectors if it changed), then embed every symbol still
    /// missing a vector. Returns how many were newly embedded. Kept free of any
    /// held SQLite connection across `.await` (the connection is not `Send`).
    async fn ensure_index(&self, dim: usize) -> anyhow::Result<usize> {
        // Sync phase: stamp model + collect what needs embedding, then drop store.
        let pending = {
            let mut store = GraphStore::open(&self.db_path, &self.root)?;
            store.refresh()?;
            store.ensure_model(self.embedder.model(), dim)?;
            store.pending_embeddings(self.max_chars)?
        };
        if pending.is_empty() {
            return Ok(0);
        }
        // Async phase: embed resiliently (no connection held). Items that survive
        // are kept aligned with their vectors; ones that can't be embedded even
        // after truncation are skipped (they stay pending, retried next call).
        let (kept_idx, vectors) = self.embed_pending(&pending).await;
        let kept: Vec<_> = kept_idx.iter().map(|&i| pending[i].clone()).collect();
        // Sync phase: persist.
        let mut store = GraphStore::open(&self.db_path, &self.root)?;
        store.store_embeddings(&kept, &vectors)?;
        Ok(kept.len())
    }

    /// Embed all pending items, batched. If a batch is rejected (typically a
    /// single dense chunk exceeding the model's context), fall back to embedding
    /// that batch item by item, halving any offender until it fits — so one
    /// oversized symbol never aborts the whole index. Returns the indices that
    /// were embedded and their vectors, parallel.
    async fn embed_pending(
        &self,
        pending: &[crate::store::EmbedItem],
    ) -> (Vec<usize>, Vec<Vec<f32>>) {
        let mut idxs = Vec::new();
        let mut vecs = Vec::new();
        for (b, chunk) in pending.chunks(EMBED_BATCH).enumerate() {
            let base = b * EMBED_BATCH;
            let texts: Vec<String> = chunk.iter().map(|i| i.text.clone()).collect();
            match self.embedder.embed(&texts).await {
                Ok(v) => {
                    for (j, vec) in v.into_iter().enumerate() {
                        idxs.push(base + j);
                        vecs.push(vec);
                    }
                }
                Err(_) => {
                    for (j, item) in chunk.iter().enumerate() {
                        if let Some(vec) = self.embed_one(&item.text).await {
                            idxs.push(base + j);
                            vecs.push(vec);
                        }
                    }
                }
            }
        }
        (idxs, vecs)
    }

    /// Embed one text, halving it on each failure until it fits the model's
    /// context or gets too small to be worth keeping.
    async fn embed_one(&self, text: &str) -> Option<Vec<f32>> {
        let mut t = text.to_string();
        for _ in 0..6 {
            match self.embedder.embed(std::slice::from_ref(&t)).await {
                Ok(mut v) => return v.pop(),
                Err(_) => {
                    let n = t.chars().count();
                    if n <= 64 {
                        return None;
                    }
                    t = t.chars().take(n / 2).collect();
                }
            }
        }
        None
    }

    /// Blend cosine similarity with graph centrality (caller count) over the
    /// candidate set. Both signals are min-max normalized, then combined 0.8/0.2
    /// so semantics leads but a well-connected symbol wins ties — the "similar ≠
    /// relevant" correction.
    fn rerank(&self, mut hits: Vec<SemanticHit>, limit: usize) -> Vec<Ranked> {
        let graph = self.graph.get();
        let cos: Vec<f32> = hits.iter().map(|h| h.score).collect();
        let callers: Vec<f32> = hits
            .iter()
            .map(|h| graph.callers(&h.name).len() as f32)
            .collect();
        let norm = |xs: &[f32]| -> Vec<f32> {
            let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
            for &x in xs {
                lo = lo.min(x);
                hi = hi.max(x);
            }
            let span = hi - lo;
            xs.iter()
                .map(|&x| if span > 0.0 { (x - lo) / span } else { 0.0 })
                .collect()
        };
        let cn = norm(&cos);
        let vn = norm(&callers);
        let mut ranked: Vec<Ranked> = hits
            .drain(..)
            .enumerate()
            .map(|(i, h)| Ranked {
                final_score: 0.8 * cn[i] + 0.2 * vn[i],
                cosine: h.score,
                callers: callers[i] as usize,
                hit: h,
            })
            .collect();
        ranked.sort_by(|a, b| {
            b.final_score
                .partial_cmp(&a.final_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        ranked.truncate(limit);
        ranked
    }
}

struct Ranked {
    final_score: f32,
    cosine: f32,
    callers: usize,
    hit: SemanticHit,
}

#[async_trait]
impl AgentTool for SemanticSearchTool {
    fn name(&self) -> &str {
        "semantic_search"
    }

    fn description(&self) -> &str {
        "Find code by MEANING, not keywords: describe what you're looking for in \
         natural language (\"where do we validate a session token\") and get the \
         most relevant symbols by embedding similarity, reranked by graph \
         centrality. Use it to explore an unfamiliar area before grep. Returns \
         file:line locations to read."
    }

    fn parameters(&self) -> Value {
        serde_json::to_value(schemars::schema_for!(SemanticArgs)).unwrap()
    }

    fn prompt_snippet(&self) -> Option<&str> {
        Some("semantic_search: find code by natural-language meaning (embeddings), then read the hits")
    }

    async fn execute(&self, ctx: ToolExecContext) -> anyhow::Result<ToolOutput> {
        let args: SemanticArgs = serde_json::from_value(ctx.args)?;
        let limit = args.limit.unwrap_or(8).clamp(1, 50);

        // Embed the query first — this also tells us the model's dimension.
        let qvecs = match self.embedder.embed(std::slice::from_ref(&args.query)).await {
            Ok(v) => v,
            Err(e) => {
                return Ok(ToolOutput::text(format!(
                    "semantic_search unavailable (embedding service error: {e}). \
                     Fall back to grep or code_map for this query."
                )));
            }
        };
        let Some(qv) = qvecs.into_iter().next() else {
            return Ok(ToolOutput::text(
                "semantic_search: embedder returned no vector for the query.",
            ));
        };
        let dim = qv.len();

        let indexed = match self.ensure_index(dim).await {
            Ok(n) => n,
            Err(e) => {
                return Ok(ToolOutput::text(format!(
                    "semantic_search: indexing failed ({e}). Use grep or code_map instead."
                )));
            }
        };

        // Search (extra candidates so the rerank has room to reorder).
        let hits = {
            let store = GraphStore::open(&self.db_path, &self.root)?;
            store.search(&qv, limit * 3)?
        };
        if hits.is_empty() {
            return Ok(ToolOutput::text(format!(
                "semantic_search: no embedded symbols matched (index has 0 vectors{}). \
                 The repo may have no supported source files.",
                if indexed > 0 {
                    format!(", {indexed} just embedded")
                } else {
                    String::new()
                }
            )));
        }
        let ranked = self.rerank(hits, limit);

        let mut out = String::new();
        if indexed > 0 {
            out.push_str(&format!("(embedded {indexed} new symbols)\n"));
        }
        out.push_str(&format!("Top {} for \"{}\":\n", ranked.len(), args.query));
        for (i, r) in ranked.iter().enumerate() {
            let rel = r
                .hit
                .file
                .strip_prefix(&self.root)
                .unwrap_or(&r.hit.file)
                .to_string_lossy();
            out.push_str(&format!(
                "{}. {} ({}:{})  [sim {:.2}, {} callers]\n",
                i + 1,
                r.hit.name,
                rel,
                r.hit.line,
                r.cosine,
                r.callers
            ));
        }
        Ok(ToolOutput::text(out))
    }
}
