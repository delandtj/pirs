//! The resilience fix caught by live testing: a small-context embedding model
//! rejects a dense chunk that exceeds its token limit. Indexing must survive by
//! truncating the offender per-item rather than aborting the whole batch.
//!
//! Drives the real `SemanticSearchTool` against a mock `/v1/embeddings` server
//! that returns HTTP 400 for any single input longer than a fixed char limit —
//! exactly how Ollama/all-minilm behaves on an over-long chunk.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;

use pirs_agent::{AgentTool, ToolExecContext};
use pirs_ai::EmbeddingClient;
use pirs_graph::semantic_search::SemanticSearchTool;
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
async fn oversized_chunk_is_truncated_not_fatal() {
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
    let tool = SemanticSearchTool::new(graph, root.clone(), db, embedder, Some(2000));

    let ctx = ToolExecContext {
        tool_call_id: "t1".into(),
        args: serde_json::json!({ "query": "tiny function", "limit": 5 }),
        cancel: tokio_util::sync::CancellationToken::new(),
        on_update: None,
    };
    let out = tool.execute(ctx).await.expect("tool must not error out");
    let text = serde_json::to_string(&out.content).unwrap();

    // The run survived and produced ranked results (not the failure note).
    assert!(
        text.contains("Top ") && text.contains("tiny"),
        "expected ranked hits including the small symbol, got: {text}"
    );
    assert!(
        !text.to_lowercase().contains("unavailable") && !text.contains("indexing failed"),
        "must not degrade to the failure path: {text}"
    );
    // Both symbols got embedded — the oversized one via truncation.
    assert!(
        text.contains("embedded 2 new symbols"),
        "both symbols embedded (huge one truncated to fit): {text}"
    );
}
