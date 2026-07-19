//! BM25 lexical search over code symbols (tantivy, in-RAM).
//!
//! The lexical arm of hybrid retrieval. Where embeddings match *meaning*, BM25
//! matches *terms* — exact identifiers, error strings, API names — which is how
//! code is most often searched, and precisely where a general embedding model is
//! weakest. Pure Rust, no model, no GPU: the index builds from the current graph
//! symbols in-memory in well under a second for thousands of symbols, so there is
//! no cold-start cost like the embedding index has.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{Schema, Value, STORED, TEXT};
use tantivy::{doc, Index, IndexReader, TantivyDocument};

use crate::Symbol;

/// One BM25 hit: which symbol matched and its relevance score.
#[derive(Debug, Clone)]
pub struct LexHit {
    pub name: String,
    pub file: PathBuf,
    pub line: usize,
    pub score: f32,
}

pub struct LexicalIndex {
    index: Index,
    reader: IndexReader,
    f_name: tantivy::schema::Field,
    f_body: tantivy::schema::Field,
    f_path: tantivy::schema::Field,
    f_line: tantivy::schema::Field,
}

impl LexicalIndex {
    /// Build an in-RAM BM25 index over `symbols`. The searchable text is the
    /// symbol name plus its source body (so both "call it by name" and "describe
    /// what it does" queries hit); path and line are stored for the result.
    pub fn build(symbols: &[Symbol], _root: &Path) -> Result<LexicalIndex> {
        let mut sb = Schema::builder();
        let f_name = sb.add_text_field("name", TEXT | STORED);
        let f_body = sb.add_text_field("body", TEXT);
        let f_path = sb.add_text_field("path", STORED);
        let f_line = sb.add_u64_field("line", STORED);
        let schema = sb.build();

        let index = Index::create_in_ram(schema);
        let mut writer = index.writer(15_000_000)?;
        let mut file_cache: HashMap<PathBuf, String> = HashMap::new();
        for s in symbols {
            let source = file_cache
                .entry(s.file.clone())
                .or_insert_with(|| std::fs::read_to_string(&s.file).unwrap_or_default());
            let body = source.get(s.start_byte..s.end_byte).unwrap_or("");
            writer.add_document(doc!(
                f_name => s.name.clone(),
                f_body => format!("{} {} {}", s.kind.name(), s.name, body),
                f_path => s.file.to_string_lossy().to_string(),
                f_line => s.line as u64,
            ))?;
        }
        writer.commit()?;
        let reader = index.reader()?;
        Ok(LexicalIndex {
            index,
            reader,
            f_name,
            f_body,
            f_path,
            f_line,
        })
    }

    /// Top-`k` symbols by BM25 over name+body. The query is reduced to bare
    /// alphanumeric terms so a natural-language question (with punctuation) never
    /// trips tantivy's query grammar.
    pub fn search(&self, query: &str, k: usize) -> Result<Vec<LexHit>> {
        let terms = sanitize(query);
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let searcher = self.reader.searcher();
        // Name matches weigh more than body matches.
        let mut qp = QueryParser::for_index(&self.index, vec![self.f_name, self.f_body]);
        qp.set_field_boost(self.f_name, 2.0);
        let query = qp.parse_query(&terms)?;
        let top = searcher.search(&query, &TopDocs::with_limit(k).order_by_score())?;
        let mut hits = Vec::with_capacity(top.len());
        for (score, addr) in top {
            let d: TantivyDocument = searcher.doc(addr)?;
            let name = d
                .get_first(self.f_name)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let path = d
                .get_first(self.f_path)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let line = d
                .get_first(self.f_line)
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            hits.push(LexHit {
                name,
                file: PathBuf::from(path),
                line,
                score,
            });
        }
        Ok(hits)
    }
}

/// Reduce an arbitrary query to space-separated alphanumeric/underscore terms.
/// Also splits camelCase/snake so `refreshStale` and `refresh_stale` both surface
/// `refresh` and `stale`.
fn sanitize(query: &str) -> String {
    let mut out = String::new();
    for tok in query.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
        if tok.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(tok);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_punctuation() {
        assert_eq!(
            sanitize("where do we refresh() the token?!"),
            "where do we refresh the token"
        );
        assert_eq!(sanitize("  "), "");
    }
}
