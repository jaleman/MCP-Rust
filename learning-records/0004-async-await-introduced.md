# Async/Await Introduced (Lesson 4)

Async/await and Tokio were introduced for the first time in Lesson 4 as part of writing the first MCP server skeleton. Covered at recognition level only — not deep understanding. Key points established:

- `async fn` returns a future; `.await` drives it to completion
- `#[tokio::main]` starts the Tokio runtime and makes main async
- The pattern `KukaServer::new().serve(stdio()).await?` is the entry point idiom for rmcp stdio servers

Also introduced: trait basics (`ServerHandler` as a contract), and rmcp's two-macro pattern (`#[tool_router]` + `#[tool_handler]`).

**Implications**: Do not re-explain what async/await or `#[tokio::main]` are from scratch. Can reference "the async runtime" without definition. Deep Tokio (tasks, channels, select!) is still unexplored — introduce explicitly when needed for PDF ingestion concurrency.
