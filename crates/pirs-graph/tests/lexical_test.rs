//! BM25 lexical search over real parsed symbols: exact-term queries must rank
//! the symbol carrying that term first — the strength BM25 brings that a general
//! embedding model lacks.

use std::fs;
use std::path::Path;

use pirs_graph::graph::parse_tree;
use pirs_graph::lexical::LexicalIndex;

const SRC: &str = r#"
fn authenticate_token(token: String) -> bool { validate(token) }
fn validate(token: String) -> bool { true }
fn render_widget(w: Widget) -> String { draw(w) }
fn draw(w: Widget) -> String { String::new() }
struct Widget { width: i32 }
"#;

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

#[test]
fn bm25_ranks_exact_term_owner_first() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "src.rs", SRC);
    let symbols = parse_tree(root);
    let idx = LexicalIndex::build(&symbols, root).unwrap();

    // Exact identifier query -> its owner ranks first.
    let hits = idx.search("authenticate_token", 5).unwrap();
    assert!(!hits.is_empty(), "expected hits");
    assert_eq!(
        hits[0].name, "authenticate_token",
        "term owner ranks first: {hits:?}"
    );

    // Conceptual-but-lexical query -> widget rendering symbols surface.
    let hits = idx.search("render widget draw", 5).unwrap();
    let names: Vec<&str> = hits.iter().map(|h| h.name.as_str()).collect();
    assert!(
        names.contains(&"render_widget") || names.contains(&"draw"),
        "widget query should surface widget symbols: {names:?}"
    );
    // And should NOT rank the unrelated auth symbol above them.
    assert_ne!(hits.first().map(|h| h.name.as_str()), Some("validate"));
}

#[test]
fn punctuation_and_empty_queries_are_safe() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "src.rs", SRC);
    let idx = LexicalIndex::build(&parse_tree(root), root).unwrap();

    // Natural-language query with punctuation must not error on tantivy grammar.
    let hits = idx.search("where do we validate() the token?!", 5).unwrap();
    assert!(
        hits.iter().any(|h| h.name == "validate"),
        "punctuated NL query works"
    );

    // Pure punctuation reduces to nothing -> empty, not an error.
    assert!(idx.search("?!.,;", 5).unwrap().is_empty());
}
