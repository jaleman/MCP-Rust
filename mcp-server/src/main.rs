// The MCP server binary. All domain logic (indexing, search, bundle loading)
// lives in the library crate — this file wires that logic to the MCP protocol:
// tool definitions, resource handlers, result formatting, and stdio transport.

use anyhow::{Context as _, Result};
use axum::Router;
use clap::Parser;
use mcp_server::bundle::resource_stem_is_safe;
use mcp_server::index::Index;
use mcp_server::search::{SearchHit, parse_query};
use rmcp::{
    // MCP SDK types: error type, server trait, service wiring, tool routing machinery,
    // parameter wrapper, all protocol model types, JSON schema + serde derives,
    // the #[tool] / #[tool_router] / #[tool_handler] macros, and stdio transport
    ErrorData as McpError,
    RoleServer,
    ServerHandler,
    ServiceExt,
    handler::server::router::tool::ToolRouter,
    handler::server::wrapper::Parameters,
    model::*,
    schemars,
    serde,
    service::RequestContext,
    tool,
    tool_handler,
    tool_router,
    transport::{
        stdio,
        streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
        },
    },
};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

#[derive(Debug, Parser)]
struct Args {
    /// Listen address for streamable HTTP, e.g. 127.0.0.1:8382. Omit for stdio.
    #[arg(long)]
    http: Option<String>,
}

/// Hard ceiling on how many documents one search_docs call formats into its
/// response. Documents are already ranked; this caps worst-case output size
/// regardless of how broadly a query matches.
const MAX_HITS_SHOWN: usize = 20;

// The input schema for the search_docs tool.
// #[derive] generates the Debug, Deserialize, and JsonSchema implementations
// automatically — equivalent to Lombok @Data + Jackson annotations in Java.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SearchInput {
    /// Search term to look up in the KUKA documentation
    query: String,
}

// The MCP server struct. The index is built once at startup (in main) and
// shared behind Arc<RwLock<…>>: Arc because the Clone derive and the MCP
// framework may hold several handles to the same server, RwLock because
// reload_docs replaces the index while other tools read it. Reads take a
// shared lock (many at once); the reload takes the exclusive write lock.
#[derive(Clone)]
struct KukaServer {
    /// Where the knowledge bundle lives — kept for read_resource and reloads.
    knowledge_dir: PathBuf,
    index: Arc<RwLock<Index>>,
    #[allow(dead_code)]
    tool_router: ToolRouter<KukaServer>,
}

// #[tool_router] scans this impl block for methods marked #[tool] and wires them
// into the MCP protocol automatically — generating the tool list and dispatch logic.
#[tool_router]
impl KukaServer {
    fn new(knowledge_dir: PathBuf, index: Index) -> Self {
        Self {
            knowledge_dir,
            index: Arc::new(RwLock::new(index)),
            tool_router: Self::tool_router(),
        }
    }

    // Lightweight health-check tool. Useful for verifying the server is reachable
    // before sending real queries.
    #[tool(description = "Ping the KUKA knowledge server to confirm it is running")]
    fn ping(&self) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::success(vec![Content::text(
            "KUKA Knowledge server is online and ready.",
        )]))
    }

    #[tool(description = "List all documents in the KUKA knowledge bundle, grouped by type")]
    fn list_docs(&self) -> Result<CallToolResult, McpError> {
        // .unwrap() on the lock: poisoning only occurs if a thread panicked
        // while holding it, in which case crashing loudly is the right move.
        Ok(format_doc_list(&self.index.read().unwrap()))
    }

    #[tool(
        description = "Search KUKA robot documentation. Always the FIRST tool for any KUKA question. \
                       Returns ranked excerpts, each with a kuka://docs/ resource URI. If the excerpts \
                       do not answer the question, retry with different search terms, or read the hit's \
                       resource URI for the full section — do not look for source files instead."
    )]
    fn search_docs(
        &self,
        Parameters(input): Parameters<SearchInput>,
    ) -> Result<CallToolResult, McpError> {
        Ok(run_search(&self.index.read().unwrap(), &input.query))
    }

    #[tool(
        description = "Rebuild the knowledge index after documents were added, removed, or re-extracted"
    )]
    fn reload_docs(&self) -> Result<CallToolResult, McpError> {
        match Index::build(&self.knowledge_dir) {
            Ok(new_index) => {
                let summary = format!(
                    "Reloaded knowledge index: {} document(s), {} unique term(s).",
                    new_index.doc_count(),
                    new_index.term_count()
                );
                // The exclusive write lock swaps the index atomically —
                // concurrent searches see either the old or the new one.
                *self.index.write().unwrap() = new_index;
                Ok(CallToolResult::success(vec![Content::text(summary)]))
            }
            // Rebuild failed (e.g. the directory vanished): report it and
            // KEEP the previous index — the server stays functional.
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Reload failed, previous index kept: {e:#}"
            ))])),
        }
    }
}

// The presentation layer for search: runs the engine (index.rs, which returns
// plain SearchHit data) and formats the outcome as an MCP tool result. All
// user-facing wording lives here, none of it in the engine.
fn run_search(index: &Index, query: &str) -> CallToolResult {
    let query_lower = query.to_lowercase();
    let terms = parse_query(&query_lower);

    // Guard: an empty term list would vacuously match nothing/everything —
    // answer with guidance instead.
    if terms.is_empty() {
        return CallToolResult::success(vec![Content::text(
            "Query contains only common words. Please add specific search terms.".to_string(),
        )]);
    }

    let hits = index.search(&terms);

    // The no-results message carries its own retry guidance: tool OUTPUT is
    // the one steering channel every harness passes to its model, so the
    // hint works even on clients that ignore MCP instructions entirely.
    let text = if hits.is_empty() {
        format!(
            "No results found for '{query}'. All search terms must match — \
             try again with fewer or different terms."
        )
    } else {
        let total = hits.len();
        let shown = &hits[..total.min(MAX_HITS_SHOWN)];
        let ranked: Vec<String> = shown.iter().map(format_hit).collect();
        let mut text = format!(
            "Found {total} result(s) for '{query}'{}:\n\n{}",
            if total > shown.len() {
                format!(", showing top {}", shown.len())
            } else {
                String::new()
            },
            ranked.join("\n\n")
        );
        if total > shown.len() {
            text.push_str(&format!(
                "\n\n…{} more result(s) omitted. Add more specific terms to narrow the query.",
                total - shown.len()
            ));
        }
        text
    };

    CallToolResult::success(vec![Content::text(text)])
}

// Renders one hit as the bullet-point block shown to the client. The pointers
// shown are kuka:// resource URIs — actions the agent can take (read the full
// section, view a diagram) — never source-file paths it might try to open.
fn format_hit(hit: &SearchHit) -> String {
    let mut out = format!("• {}\n  Resource: kuka://docs/{}", hit.title, hit.stem);
    if !hit.images.is_empty() {
        let uris: Vec<String> = hit
            .images
            .iter()
            .map(|image| format!("kuka://images/{image}"))
            .collect();
        out.push_str(&format!("\n  Diagrams: {}", uris.join(", ")));
    }
    out.push_str(&format!("\n\n  ...{}...", hit.excerpts.join("\n\n  ...")));
    out
}

// Renders the document listing from index metadata — no disk access at all.
fn format_doc_list(index: &Index) -> CallToolResult {
    let docs = index.docs();

    if docs.is_empty() {
        return CallToolResult::success(vec![Content::text(
            "No documents found in the knowledge bundle.".to_string(),
        )]);
    }

    // Group titles by document type
    let mut grouped: HashMap<&str, Vec<&str>> = HashMap::new();
    for doc in docs {
        grouped.entry(&doc.doc_type).or_default().push(&doc.title);
    }

    // Sort the type keys alphabetically so the output is stable across runs
    let mut type_keys: Vec<&str> = grouped.keys().copied().collect();
    type_keys.sort_unstable();

    // Build one text section per type, with titles sorted within each section
    let sections: Vec<String> = type_keys
        .into_iter()
        .map(|doc_type| {
            let mut titles = grouped.remove(doc_type).unwrap_or_default();
            titles.sort_unstable();
            let items: Vec<String> = titles.into_iter().map(|t| format!("  • {t}")).collect();
            format!("{doc_type}:\n{}", items.join("\n"))
        })
        .collect();

    CallToolResult::success(vec![Content::text(format!(
        "Knowledge bundle — {} document(s):\n\n{}",
        docs.len(),
        sections.join("\n\n")
    ))])
}

// #[tool_handler] wires the tool_router into the ServerHandler trait so the MCP
// framework knows how to dispatch incoming tool calls.
#[tool_handler]
impl ServerHandler for KukaServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_instructions(
            "KUKA AMR robot knowledge server, grounded in official KUKA \
             documentation (AMR fleet manuals, technical notes, safety and \
             deployment guides). Recommended workflow for ANY KUKA question: \
             (1) call search_docs first; (2) if the excerpts do not fully \
             answer, retry search_docs with different terms, or read the \
             kuka://docs/{name} resource shown in the hit to get the full \
             section (sections are small — always safe to read whole). Hits \
             may also list Diagrams: kuka://images/{name} resources — read \
             one to view the diagram referenced by that text. Do \
             not browse the resource list to hunt for answers, and never \
             fall back to reading source documents outside these tools. \
             list_docs shows every document grouped by type. After \
             re-extracting documentation, call reload_docs to rebuild the \
             index. ping confirms the server is alive."
                .to_string(),
        )
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        // Straight from index metadata — no disk access.
        let index = self.index.read().unwrap();
        let mut resources: Vec<Resource> = index
            .docs()
            .iter()
            .map(|doc| {
                let uri = format!("kuka://docs/{}", doc.stem);
                let mut raw = RawResource::new(uri, doc.stem.clone())
                    .with_title(doc.title.clone())
                    .with_mime_type("text/markdown".to_string());
                if let Some(desc) = &doc.description {
                    raw = raw.with_description(desc.clone());
                }
                Annotated::new(raw, None)
            })
            .collect();

        // Diagrams extracted from the documents, served as image resources
        for doc in index.docs() {
            for image in &doc.images {
                let uri = format!("kuka://images/{image}");
                let raw = RawResource::new(uri, image.clone())
                    .with_title(format!("Diagram from {}", doc.title))
                    .with_mime_type("image/png".to_string());
                resources.push(Annotated::new(raw, None));
            }
        }

        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        let uri = &request.uri;

        // Diagram resources: PNG bytes served base64-encoded as an MCP blob.
        // Multimodal clients render/interpret these directly.
        if let Some(image_name) = uri.strip_prefix("kuka://images/") {
            // Same traversal guard as documents — plain filenames only
            if !resource_stem_is_safe(image_name) {
                return Err(McpError {
                    code: ErrorCode::RESOURCE_NOT_FOUND,
                    message: format!("Invalid resource URI: {uri}").into(),
                    data: None,
                });
            }
            let path = self.knowledge_dir.join("images").join(image_name);
            let bytes = std::fs::read(&path).map_err(|_| McpError {
                code: ErrorCode::RESOURCE_NOT_FOUND,
                message: format!("Resource not found: {uri}").into(),
                data: None,
            })?;

            use base64::Engine as _;
            let blob = base64::engine::general_purpose::STANDARD.encode(&bytes);
            let contents = ResourceContents::blob(blob, uri.clone()).with_mime_type("image/png");
            return Ok(ReadResourceResult::new(vec![contents]));
        }

        // Strip the kuka://docs/ prefix to recover the file stem
        let stem = uri.strip_prefix("kuka://docs/").ok_or_else(|| McpError {
            code: ErrorCode::RESOURCE_NOT_FOUND,
            message: format!("Unknown resource URI: {uri}").into(),
            data: None,
        })?;

        // Path-traversal guard: a stem like "../../secret" would escape the
        // knowledge directory when joined below. Only plain stems are valid.
        if !resource_stem_is_safe(stem) {
            return Err(McpError {
                code: ErrorCode::RESOURCE_NOT_FOUND,
                message: format!("Invalid resource URI: {uri}").into(),
                data: None,
            });
        }

        let path = self.knowledge_dir.join(format!("{stem}.md"));
        let content = std::fs::read_to_string(&path).map_err(|_| McpError {
            code: ErrorCode::RESOURCE_NOT_FOUND,
            message: format!("Resource not found: {uri}").into(),
            data: None,
        })?;

        Ok(ReadResourceResult::new(vec![ResourceContents::text(
            content,
            uri.clone(),
        )]))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Send tracing output to stderr so it doesn't mix with MCP's stdout messages.
    // Log level is controlled by the RUST_LOG environment variable at runtime.
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .with_ansi(false) // disable colour codes (they corrupt MCP's JSON stream)
        .init();

    // Resolve configuration exactly ONCE, here. Reads KUKA_KNOWLEDGE_DIR if
    // set; falls back to "knowledge" relative to the working directory.
    let knowledge_dir = PathBuf::from(
        std::env::var("KUKA_KNOWLEDGE_DIR").unwrap_or_else(|_| "knowledge".to_string()),
    );

    // Build the index up front. Bad configuration (missing directory) stops
    // the server at startup with a loud error — the composition root is the
    // right place to fail fast.
    let started = std::time::Instant::now();
    let index = Index::build(&knowledge_dir).with_context(|| {
        format!(
            "failed to build knowledge index from {}",
            knowledge_dir.display()
        )
    })?;

    tracing::info!(
        "Starting KUKA MCP server: indexed {} document(s), {} unique term(s) in {:.1?} (knowledge dir: {})",
        index.doc_count(),
        index.term_count(),
        started.elapsed(),
        knowledge_dir.display()
    );

    let server = KukaServer::new(knowledge_dir, index);
    match args.http {
        None => {
            // Attach the server to stdin/stdout and block until the client disconnects.
            let service = server.serve(stdio()).await?;
            service.waiting().await?;
        }
        Some(addr) => serve_http(addr, server).await?,
    }
    Ok(())
}

async fn serve_http(addr: String, server: KukaServer) -> Result<()> {
    let socket_addr: SocketAddr = addr
        .parse()
        .with_context(|| format!("invalid --http listen address: {addr}"))?;

    // HTTP mode has no authentication in this step. Keep the normal path on
    // loopback; warn loudly if someone chooses a public bind address.
    if matches!(socket_addr.ip(), IpAddr::V4(ip) if ip.is_unspecified())
        || matches!(socket_addr.ip(), IpAddr::V6(ip) if ip.is_unspecified())
    {
        tracing::warn!(
            "HTTP mode has no authentication; binding to {addr} may expose the MCP server. Prefer 127.0.0.1 unless this is protected by a tunnel or firewall."
        );
    }

    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    let app = Router::new().route_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind(socket_addr)
        .await
        .with_context(|| format!("failed to bind HTTP listener at {addr}"))?;

    tracing::info!("KUKA MCP server listening on http://{addr}/mcp");
    axum::serve(listener, app).await?;
    Ok(())
}

// Tests for the PRESENTATION layer: wording, isError flags, guards, and the
// reload tool. The engine itself is tested in the library (index.rs); these
// only check what this binary adds on top. Note: the library's #[cfg(test)]
// test_util is not visible here — a binary is a separate crate, and the lib
// it links against is compiled without cfg(test) — so this module builds its
// own fixture.
#[cfg(test)]
mod tool_tests {
    use super::*;
    use std::fs;

    #[test]
    fn args_default_to_stdio() {
        let args = Args::try_parse_from(["mcp-server"]).unwrap();
        assert_eq!(args.http, None);
    }

    #[test]
    fn args_accept_http_listen_address() {
        let args = Args::try_parse_from(["mcp-server", "--http", "127.0.0.1:8382"]).unwrap();
        assert_eq!(args.http.as_deref(), Some("127.0.0.1:8382"));
    }

    fn bundle_with_one_doc() -> tempfile::TempDir {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let doc = "\
---
type: technical-note
title: Reflector Guide
description: Fixture for tool-layer tests.
resource: kuka-docs/test.pdf
tags: [test]
timestamp: 2026-01-01T00:00:00Z
---

Reflectors must be mounted at a height of 150 to 2000 mm above floor level.";
        fs::write(temp_dir.path().join("reflector-guide.md"), doc).unwrap();
        temp_dir
    }

    fn server_over(temp_dir: &tempfile::TempDir) -> KukaServer {
        let index = Index::build(temp_dir.path()).unwrap();
        KukaServer::new(temp_dir.path().to_path_buf(), index)
    }

    // Pulls the text out of a CallToolResult for wording assertions.
    fn result_text(result: &CallToolResult) -> String {
        format!("{:?}", result.content)
    }

    #[test]
    fn search_tool_formats_hits() {
        let temp_dir = bundle_with_one_doc();
        let server = server_over(&temp_dir);

        let result = server
            .search_docs(Parameters(SearchInput {
                query: "reflector height".to_string(),
            }))
            .unwrap();

        assert_ne!(result.is_error, Some(true));
        let text = result_text(&result);
        assert!(text.contains("Found 1 result(s) for 'reflector height'"));
        assert!(text.contains("Reflector Guide"));
        assert!(
            text.contains("Resource: kuka://docs/reflector-guide"),
            "hits must carry the actionable resource URI, not a source-file path"
        );
        assert!(
            !text.contains(".pdf"),
            "no source-file paths in tool output"
        );
    }

    #[test]
    fn search_tool_rejects_stop_word_only_query() {
        let temp_dir = bundle_with_one_doc();
        let server = server_over(&temp_dir);
        let result = server
            .search_docs(Parameters(SearchInput {
                query: "what is the".to_string(),
            }))
            .unwrap();
        assert_ne!(result.is_error, Some(true), "guard message is not an error");
        assert!(result_text(&result).contains("only common words"));
    }

    #[test]
    fn search_tool_reports_no_results_with_retry_hint() {
        let temp_dir = bundle_with_one_doc();
        let server = server_over(&temp_dir);
        let result = server
            .search_docs(Parameters(SearchInput {
                query: "hydraulic pump".to_string(),
            }))
            .unwrap();
        let text = result_text(&result);
        assert!(text.contains("No results found for 'hydraulic pump'"));
        // In-band steering: the retry guidance rides inside the tool result,
        // so it reaches agents on ANY harness, not just clients that read
        // MCP instructions.
        assert!(text.contains("fewer or different terms"));
    }

    #[test]
    fn search_tool_caps_hit_count_with_trailer() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        for i in 0..(MAX_HITS_SHOWN + 5) {
            let doc = format!(
                "---\ntype: technical-note\ntitle: Shared Topic {i:02}\nresource: kuka-docs/{i:02}.pdf\n---\n\nCommon bounded output topic."
            );
            fs::write(temp_dir.path().join(format!("shared-topic-{i:02}.md")), doc).unwrap();
        }
        let server = server_over(&temp_dir);

        let result = server
            .search_docs(Parameters(SearchInput {
                query: "common topic".to_string(),
            }))
            .unwrap();

        let text = result_text(&result);
        assert!(text.contains("Found 25 result(s) for 'common topic', showing top 20"));
        assert_eq!(text.matches('•').count(), MAX_HITS_SHOWN);
        assert!(text.contains("…5 more result(s) omitted"));
        assert!(text.contains("Add more specific terms"));
    }

    #[test]
    fn list_docs_formats_from_index_metadata() {
        let temp_dir = bundle_with_one_doc();
        let server = server_over(&temp_dir);
        let result = server.list_docs().unwrap();
        let text = result_text(&result);
        assert!(text.contains("Knowledge bundle — 1 document(s)"));
        assert!(text.contains("technical-note"));
        assert!(text.contains("Reflector Guide"));
    }

    #[test]
    fn reload_docs_picks_up_new_documents() {
        let temp_dir = bundle_with_one_doc();
        let server = server_over(&temp_dir);

        // Not indexed yet — added after the server started
        let doc = "\
---
type: technical-note
title: Brand New Note
resource: kuka-docs/new.pdf
---

Hydraulic pumps are not a KUKA topic, but this note mentions them.";
        fs::write(temp_dir.path().join("brand-new.md"), doc).unwrap();

        let before = server
            .search_docs(Parameters(SearchInput {
                query: "hydraulic".to_string(),
            }))
            .unwrap();
        assert!(result_text(&before).contains("No results found"));

        let reload = server.reload_docs().unwrap();
        assert!(result_text(&reload).contains("2 document(s)"));

        let after = server
            .search_docs(Parameters(SearchInput {
                query: "hydraulic".to_string(),
            }))
            .unwrap();
        assert!(result_text(&after).contains("Brand New Note"));
    }

    #[test]
    fn reload_failure_keeps_previous_index() {
        // Server built over a SUBDIRECTORY which is then deleted: the reload
        // must fail loudly but the old index keeps answering queries.
        let outer = tempfile::TempDir::new().unwrap();
        let bundle_dir = outer.path().join("bundle");
        fs::create_dir(&bundle_dir).unwrap();
        let doc = "\
---
type: technical-note
title: Survivor Note
resource: kuka-docs/s.pdf
---

Reflectors must be mounted at a height of 150 mm.";
        fs::write(bundle_dir.join("survivor.md"), doc).unwrap();

        let index = Index::build(&bundle_dir).unwrap();
        let server = KukaServer::new(bundle_dir.clone(), index);

        fs::remove_dir_all(&bundle_dir).unwrap();

        let reload = server.reload_docs().unwrap();
        assert_eq!(
            reload.is_error,
            Some(true),
            "reload of a vanished dir must error"
        );
        assert!(result_text(&reload).contains("previous index kept"));

        // The old index still answers (excerpt read fails silently — the
        // file is gone — but the hit itself must survive)
        let result = server
            .search_docs(Parameters(SearchInput {
                query: "reflector".to_string(),
            }))
            .unwrap();
        assert!(result_text(&result).contains("Survivor Note"));
    }
}
