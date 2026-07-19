//! The resilience fix caught by live testing: a small-context embedding model
//! rejects a dense chunk that exceeds its token limit. The semantic arm of
//! `code_search` must survive by truncating the offender per-item rather than
//! aborting the whole batch — and the tool must always return BM25 results even
//! when the embedding service misbehaves.
//!
//! Drives the real `CodeSearchTool` against a mock `/v1/embeddings` server that
//! returns HTTP 400 for any single input longer than a fixed char limit —
//! exactly how Ollama/all-minilm behaves on an over-long chunk.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;

use pirs_agent::{AgentTool, ToolExecContext};
use pirs_ai::EmbeddingClient;
use pirs_graph::code_search::CodeSearchTool;
use pirs_graph::LazyGraph;

/// A mock embeddings server: for each string in `input`, return a 4-dim vector
/// unless it exceeds `char_limit`, in which case fail the whole request 400 with
/// a context-length message (mirroring the real model). Serves `conns` requests.
fn spawn_embedder(char_limit: usize, conns: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for _ in 0..conns {
            let Ok((mut sock, _)) = listener.accept() else {
                break;
            };
            let mut buf = vec![0u8; 65536];
            let n = sock.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            let body = req.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
            let v: serde_json::Value =
                serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
            let inputs = v
                .get("input")
                .and_then(|i| i.as_array())
                .cloned()
                .unwrap_or_default();
            let over = inputs.iter().any(|s| {
                s.as_str()
                    .map(|t| t.chars().count() > char_limit)
                    .unwrap_or(false)
            });
            let (status, payload) = if over {
                (
                    "400 Bad Request",
                    r#"{"error":{"message":"the input length exceeds the context length"}}"#
                        .to_string(),
                )
            } else {
                let data: Vec<serde_json::Value> = inputs
                    .iter()
                    .enumerate()
                    .map(
                        |(i, _)| serde_json::json!({"index": i, "embedding": [0.1, 0.2, 0.3, 0.4]}),
                    )
                    .collect();
                ("200 OK", serde_json::json!({ "data": data }).to_string())
            };
            let resp = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                payload.len()
            );
            let _ = sock.write_all(resp.as_bytes());
        }
    });
    format!("http://{addr}/v1")
}

fn write(root: &std::path::Path, rel: &str, body: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
}

#[tokio::test]
async fn oversized_chunk_survives_and_semantic_arm_activates() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();

    // One small symbol (embeds fine) and one huge one (2400 chars > limit) that
    // must be truncated to fit rather than sinking the whole index.
    write(&root, "small.rs", "fn tiny() -> i32 { 1 }\n");
    let big_body = "x".repeat(2400);
    write(
        &root,
        "big.rs",
        &format!("fn huge() -> i32 {{ /* {big_body} */ 2 }}\n"),
    );

    // Mock model with a 200-char context; plenty of connections for the fallback's
    // per-item + truncation retries.
    let base = spawn_embedder(200, 200);
    let embedder = EmbeddingClient::new(base, "mock-mini", None);

    let db = root.join(".pirs/graph.db");
    let graph = Arc::new(LazyGraph::persistent(root.clone(), db.clone()));
    let tool = CodeSearchTool::new(
        graph,
        root.clone(),
        db,
        Some(embedder),
        Some(2000),
        Some(256),
    );

    let ctx = ToolExecContext {
        tool_call_id: "t1".into(),
        args: serde_json::json!({ "query": "tiny function", "limit": 5 }),
        cancel: tokio_util::sync::CancellationToken::new(),
        on_update: None,
    };
    let out = tool.execute(ctx).await.expect("tool must not error out");
    let text = serde_json::to_string(&out.content).unwrap();

    // BM25 always finds the lexical match, regardless of the embedding service.
    assert!(
        text.contains("Top ") && text.contains("tiny"),
        "expected ranked hits including the small symbol, got: {text}"
    );
    // The semantic arm activated: both symbols embedded (huge one via truncation),
    // so the fused result reports semantic participation rather than degrading.
    assert!(
        text.contains("semantic"),
        "semantic arm should be active after both symbols embedded: {text}"
    );
}

#[tokio::test]
async fn embedding_service_down_falls_back_to_lexical() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    write(
        &root,
        "auth.rs",
        "fn authenticate_token(t: String) -> bool { true }\n",
    );

    // Point at a dead port: every embed call fails, so the semantic arm is inert.
    let embedder = EmbeddingClient::new("http://127.0.0.1:1/v1".to_string(), "dead", None);
    let db = root.join(".pirs/graph.db");
    let graph = Arc::new(LazyGraph::persistent(root.clone(), db.clone()));
    let tool = CodeSearchTool::new(
        graph,
        root.clone(),
        db,
        Some(embedder),
        Some(2000),
        Some(64),
    );

    let ctx = ToolExecContext {
        tool_call_id: "t1".into(),
        args: serde_json::json!({ "query": "authenticate_token", "limit": 5 }),
        cancel: tokio_util::sync::CancellationToken::new(),
        on_update: None,
    };
    let out = tool
        .execute(ctx)
        .await
        .expect("must not error when service down");
    let text = serde_json::to_string(&out.content).unwrap();
    assert!(
        text.contains("authenticate_token"),
        "lexical result survives a dead embedding service: {text}"
    );
    assert!(
        text.contains("semantic index empty") || text.contains("lexical+graph"),
        "should report lexical-only fallback: {text}"
    );
}
