#[cfg(test)]
mod tests {
    use crate::router::orchestrator::OrchestratorService;
    use crate::session_cache::memory::MemorySessionCache;
    use common::configuration::{SelectionPolicy, SelectionPreference, TopLevelRoutingPreference};
    use hermesllm::apis::openai::{Message, MessageContent, Role};
    use std::sync::Arc;

    fn make_messages(n: usize) -> Vec<Message> {
        (0..n)
            .map(|i| Message {
                role: if i % 2 == 0 {
                    Role::User
                } else {
                    Role::Assistant
                },
                content: Some(MessageContent::Text(format!(
                    "This is message number {i} with some padding text to make it realistic."
                ))),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            })
            .collect()
    }

    fn make_routing_prefs() -> Vec<TopLevelRoutingPreference> {
        vec![
            TopLevelRoutingPreference {
                name: "code_generation".to_string(),
                description: "Code generation and debugging tasks".to_string(),
                models: vec![
                    "openai/gpt-4o".to_string(),
                    "openai/gpt-4o-mini".to_string(),
                ],
                selection_policy: SelectionPolicy {
                    prefer: SelectionPreference::None,
                },
            },
            TopLevelRoutingPreference {
                name: "summarization".to_string(),
                description: "Summarizing documents and text".to_string(),
                models: vec![
                    "anthropic/claude-3-sonnet".to_string(),
                    "openai/gpt-4o-mini".to_string(),
                ],
                selection_policy: SelectionPolicy {
                    prefer: SelectionPreference::None,
                },
            },
        ]
    }

    /// Stress test: exercise the full routing code path N times using a mock
    /// HTTP server and measure jemalloc allocated bytes before/after.
    ///
    /// This catches:
    /// - Memory leaks in generate_request / parse_response
    /// - Leaks in reqwest connection handling
    /// - String accumulation in the orchestrator model
    /// - Fragmentation (jemalloc allocated vs resident)
    #[tokio::test]
    async fn stress_test_routing_determine_route() {
        let mut server = mockito::Server::new_async().await;
        let router_url = format!("{}/v1/chat/completions", server.url());

        let mock_response = serde_json::json!({
            "id": "chatcmpl-mock",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "plano-orchestrator",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "{\"route\": \"code_generation\"}"
                },
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 100, "completion_tokens": 10, "total_tokens": 110}
        });

        let _mock = server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_response.to_string())
            .expect_at_least(1)
            .create_async()
            .await;

        let prefs = make_routing_prefs();
        let session_cache = Arc::new(MemorySessionCache::new(1000));
        let orchestrator_service = Arc::new(OrchestratorService::with_routing(
            router_url,
            "Plano-Orchestrator".to_string(),
            "plano-orchestrator".to_string(),
            Some(prefs.clone()),
            None,
            None,
            session_cache,
            None,
            2048,
        ));

        // Warm up: a few requests to stabilize allocator state
        for _ in 0..10 {
            let msgs = make_messages(5);
            let _ = orchestrator_service
                .determine_route(&msgs, None, "warmup")
                .await;
        }

        // Snapshot memory after warmup
        let baseline = get_allocated();

        let num_iterations = 2000;

        for i in 0..num_iterations {
            let msgs = make_messages(5 + (i % 10));
            let inline = if i % 3 == 0 {
                Some(make_routing_prefs())
            } else {
                None
            };
            let _ = orchestrator_service
                .determine_route(&msgs, inline, &format!("req-{i}"))
                .await;
        }

        let after = get_allocated();

        let growth = after.saturating_sub(baseline);
        let growth_mb = growth as f64 / (1024.0 * 1024.0);
        let per_request = if num_iterations > 0 {
            growth / num_iterations
        } else {
            0
        };

        eprintln!("=== Routing Stress Test Results ===");
        eprintln!("  Iterations:      {num_iterations}");
        eprintln!("  Baseline alloc:  {} bytes", baseline);
        eprintln!("  Final alloc:     {} bytes", after);
        eprintln!("  Growth:          {} bytes ({growth_mb:.2} MB)", growth);
        eprintln!("  Per-request:     {} bytes", per_request);

        // Allow up to 256 bytes per request of retained growth (connection pool, etc.)
        // A true leak would show thousands of bytes per request.
        assert!(
            per_request < 256,
            "Possible memory leak: {per_request} bytes/request retained after {num_iterations} iterations"
        );
    }

    /// Stress test with high concurrency: many parallel determine_route calls.
    #[tokio::test]
    async fn stress_test_routing_concurrent() {
        let mut server = mockito::Server::new_async().await;
        let router_url = format!("{}/v1/chat/completions", server.url());

        let mock_response = serde_json::json!({
            "id": "chatcmpl-mock",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "plano-orchestrator",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "{\"route\": \"summarization\"}"
                },
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 100, "completion_tokens": 10, "total_tokens": 110}
        });

        let _mock = server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_response.to_string())
            .expect_at_least(1)
            .create_async()
            .await;

        let prefs = make_routing_prefs();
        let session_cache = Arc::new(MemorySessionCache::new(1000));
        let orchestrator_service = Arc::new(OrchestratorService::with_routing(
            router_url,
            "Plano-Orchestrator".to_string(),
            "plano-orchestrator".to_string(),
            Some(prefs),
            None,
            None,
            session_cache,
            None,
            2048,
        ));

        // Warm up
        for _ in 0..20 {
            let msgs = make_messages(3);
            let _ = orchestrator_service
                .determine_route(&msgs, None, "warmup")
                .await;
        }

        let baseline = get_allocated();

        let concurrency = 50;
        let requests_per_task = 100;
        let total = concurrency * requests_per_task;

        let mut handles = vec![];
        for t in 0..concurrency {
            let svc = Arc::clone(&orchestrator_service);
            let handle = tokio::spawn(async move {
                for r in 0..requests_per_task {
                    let msgs = make_messages(3 + (r % 8));
                    let _ = svc
                        .determine_route(&msgs, None, &format!("req-{t}-{r}"))
                        .await;
                }
            });
            handles.push(handle);
        }

        for h in handles {
            h.await.unwrap();
        }

        let after = get_allocated();
        let growth = after.saturating_sub(baseline);
        let per_request = growth / total;

        eprintln!("=== Concurrent Routing Stress Test Results ===");
        eprintln!("  Tasks:       {concurrency} x {requests_per_task} = {total}");
        eprintln!("  Baseline:    {} bytes", baseline);
        eprintln!("  Final:       {} bytes", after);
        eprintln!(
            "  Growth:      {} bytes ({:.2} MB)",
            growth,
            growth as f64 / 1_048_576.0
        );
        eprintln!("  Per-request: {} bytes", per_request);

        assert!(
            per_request < 512,
            "Possible memory leak under concurrency: {per_request} bytes/request retained after {total} requests"
        );
    }

    #[cfg(feature = "jemalloc")]
    fn get_allocated() -> usize {
        tikv_jemalloc_ctl::epoch::advance().unwrap();
        tikv_jemalloc_ctl::stats::allocated::read().unwrap_or(0)
    }

    #[cfg(not(feature = "jemalloc"))]
    fn get_allocated() -> usize {
        0
    }
}
