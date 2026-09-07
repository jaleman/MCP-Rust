// The MCP server binary. All domain logic (indexing, search, bundle loading)
// lives in the library crate — this file wires that logic to the MCP protocol:
// tool definitions, resource handlers, result formatting, and stdio transport.

use anyhow::{Context as _, Result};
use axum::Router;
use clap::Parser;
use mcp_server::bundle::{media_path_is_safe, resource_stem_is_safe};
use mcp_server::index::Index;
use mcp_server::media::MediaRegistry;
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

    /// Additional `Host` header value to accept (repeatable), e.g. a public
    /// tunnel hostname (`kuka-mcp.example.dev`) or `host:port`. Loopback
    /// hosts (localhost, 127.0.0.1, ::1) are always accepted regardless.
    #[arg(long = "allowed-host")]
    allowed_host: Vec<String>,
}

/// Hard ceiling on how many documents one search_docs call formats into its
/// response. Documents are already ranked; this caps worst-case output size
/// regardless of how broadly a query matches.
const MAX_HITS_SHOWN: usize = 20;

/// Maximum size for binary blobs served via resources/read (10 MB).
/// Files larger than this (e.g. video files) return an McpError instructing
/// the client to query metadata via get_media or access the file directly.
const MAX_MEDIA_BLOB_SIZE: u64 = 10 * 1024 * 1024;

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

// The input schema for the search_docs tool.
// #[derive] generates the Debug, Deserialize, and JsonSchema implementations
// automatically — equivalent to Lombok @Data + Jackson annotations in Java.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SearchInput {
    /// Search term to look up in the KUKA documentation
    query: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ListMediaInput {
    /// Filter media resources by topic (e.g. electrical, mechanical, safety, vision)
    topic: Option<String>,
    /// Filter media resources by keyword or query term
    keyword: Option<String>,
    /// Filter media resources by asset type (e.g. video, print)
    #[serde(rename = "type")]
    media_type: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct GetMediaInput {
    /// Fetch a media resource by its unique identifier (e.g. charger-schematics-ady6102)
    id: Option<String>,
    /// Fetch a media resource matching a keyword
    keyword: Option<String>,
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
    workspace_dir: PathBuf,
    index: Arc<RwLock<Index>>,
    media_registry: Arc<RwLock<MediaRegistry>>,
    #[allow(dead_code)]
    tool_router: ToolRouter<KukaServer>,
}

// #[tool_router] scans this impl block for methods marked #[tool] and wires them
// into the MCP protocol automatically — generating the tool list and dispatch logic.
#[tool_router]
impl KukaServer {
    fn new(
        knowledge_dir: PathBuf,
        workspace_dir: PathBuf,
        index: Index,
        media_registry: MediaRegistry,
    ) -> Self {
        Self {
            knowledge_dir,
            workspace_dir,
            index: Arc::new(RwLock::new(index)),
            media_registry: Arc::new(RwLock::new(media_registry)),
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
        description = "Search KUKA robot text documentation. Always the FIRST tool for text manual questions. \
                       Returns ranked excerpts, each with a kuka://docs/ resource URI. If searching for \
                       videos, print PDFs, schematics, or drawings, use list_media instead."
    )]
    fn search_docs(
        &self,
        Parameters(input): Parameters<SearchInput>,
    ) -> Result<CallToolResult, McpError> {
        Ok(run_search(&self.index.read().unwrap(), &input.query))
    }

    #[tool(
        description = "List media resources (videos, print PDFs, schematics) matching topic, keyword, or asset type"
    )]
    fn list_media(
        &self,
        Parameters(input): Parameters<ListMediaInput>,
    ) -> Result<CallToolResult, McpError> {
        let registry = self.media_registry.read().unwrap();
        let items = registry.list_media(
            input.topic.as_deref(),
            input.keyword.as_deref(),
            input.media_type.as_deref(),
        );

        if items.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No media resources found matching the specified criteria.",
            )]));
        }

        let formatted: Vec<String> = items
            .iter()
            .map(|item| {
                let mut line = format!(
                    "• {} [{}]\n  ID: {}\n  Topic: {}\n  Resource: {}\n  Keywords: {}",
                    item.title,
                    item.media_type,
                    item.id,
                    item.topic,
                    item.uri,
                    item.keywords.join(", ")
                );
                if let Some(desc) = &item.description {
                    line.push_str(&format!("\n  Description: {desc}"));
                }
                line
            })
            .collect();

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Found {} media resource(s):\n\n{}",
            items.len(),
            formatted.join("\n\n")
        ))]))
    }

    #[tool(
        description = "Fetch details for a specific media item (video, print PDF) by id or keyword"
    )]
    fn get_media(
        &self,
        Parameters(input): Parameters<GetMediaInput>,
    ) -> Result<CallToolResult, McpError> {
        let registry = self.media_registry.read().unwrap();
        let item = registry.get_media(input.id.as_deref(), input.keyword.as_deref());

        match item {
            Some(item) => {
                let mut out = format!(
                    "Media Item: {}\n  ID: {}\n  Type: {}\n  Topic: {}\n  MIME Type: {}\n  Resource: {}\n  Filename: {}\n  Keywords: {}",
                    item.title,
                    item.id,
                    item.media_type,
                    item.topic,
                    item.mime_type,
                    item.uri,
                    item.filename,
                    item.keywords.join(", ")
                );
                if let Some(desc) = &item.description {
                    out.push_str(&format!("\n  Description: {desc}"));
                }
                Ok(CallToolResult::success(vec![Content::text(out)]))
            }
            None => Ok(CallToolResult::error(vec![Content::text(
                "Media resource not found matching the provided id or keyword.",
            )])),
        }
    }

    #[tool(
        description = "Rebuild the knowledge index and reload media registries after documents or media were added or updated"
    )]
    fn reload_docs(&self) -> Result<CallToolResult, McpError> {
        let new_media = MediaRegistry::load(&self.workspace_dir).unwrap_or_default();
        let media_count = new_media.items().len();
        *self.media_registry.write().unwrap() = new_media;

        match Index::build(&self.knowledge_dir) {
            Ok(new_index) => {
                let summary = format!(
                    "Reloaded knowledge index: {} document(s), {} unique term(s); {} media item(s).",
                    new_index.doc_count(),
                    new_index.term_count(),
                    media_count
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

impl KukaServer {
    fn read_doc_resource_content(&self, stem: &str, uri: &str) -> Result<String, McpError> {
        let path = self.knowledge_dir.join(format!("{stem}.md"));
        let mut content = std::fs::read_to_string(&path).map_err(|_| McpError {
            code: ErrorCode::RESOURCE_NOT_FOUND,
            message: format!("Resource not found: {uri}").into(),
            data: None,
        })?;

        let continues = {
            let index = self.index.read().unwrap();
            index
                .docs()
                .iter()
                .find(|doc| doc.stem == stem)
                .and_then(|doc| doc.next_stem.clone())
        };
        if let Some(next) = continues {
            content.push_str(&format!(
                "\n\n---\n[This section continues in kuka://docs/{next}]"
            ));
        }

        Ok(content)
    }

    fn read_media_resource_contents(
        &self,
        relative_path: &str,
        uri: &str,
    ) -> Result<ResourceContents, McpError> {
        if !media_path_is_safe(relative_path) {
            return Err(McpError {
                code: ErrorCode::RESOURCE_NOT_FOUND,
                message: format!("Invalid resource URI: {uri}").into(),
                data: None,
            });
        }
        if relative_path.starts_with("kuka-movies/") {
            let registry = self.media_registry.read().unwrap();
            let meta = registry.items().iter().find(|item| item.uri == *uri);

            let item = match meta {
                Some(item) => item,
                None => {
                    return Err(McpError {
                        code: ErrorCode::RESOURCE_NOT_FOUND,
                        message: format!("Media resource not found: {uri}").into(),
                        data: None,
                    });
                }
            };

            let mut info = format!(
                "Video binary content is not served over MCP.\n\nFile: {relative_path}\nTitle: {}\nTopic: {}\nKeywords: {}\n",
                item.title,
                item.topic,
                item.keywords.join(", ")
            );
            if let Some(desc) = &item.description {
                info.push_str(&format!("Description: {desc}\n"));
            }
            info.push_str("\nUse get_media or list_media to query video asset metadata.");
            return Ok(ResourceContents::text(info, uri.to_string()).with_mime_type("text/plain"));
        }
        let file_path = self.workspace_dir.join(relative_path);
        if file_path.exists() && file_path.is_file() {
            let file_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);
            if file_size > MAX_MEDIA_BLOB_SIZE {
                let size_str = format_bytes(file_size);
                let cap_str = format_bytes(MAX_MEDIA_BLOB_SIZE);
                let info = format!(
                    "Media resource '{uri}' ({size_str}) exceeds the {cap_str} limit for direct binary transfer.\n\n\
                     File: {relative_path}\n\
                     Use get_media or list_media for asset metadata."
                );
                return Ok(
                    ResourceContents::text(info, uri.to_string()).with_mime_type("text/plain")
                );
            }

            let bytes = std::fs::read(&file_path).map_err(|_| McpError {
                code: ErrorCode::RESOURCE_NOT_FOUND,
                message: format!("Resource read failed: {uri}").into(),
                data: None,
            })?;

            let registry = self.media_registry.read().unwrap();
            let mime_type = registry
                .items()
                .iter()
                .find(|item| item.uri == *uri)
                .map(|item| item.mime_type.clone())
                .unwrap_or_else(|| "application/octet-stream".to_string());

            use base64::Engine as _;
            let blob = base64::engine::general_purpose::STANDARD.encode(&bytes);
            Ok(ResourceContents::blob(blob, uri.to_string()).with_mime_type(mime_type))
        } else {
            Err(McpError {
                code: ErrorCode::RESOURCE_NOT_FOUND,
                message: format!("Media file not found on disk: {uri}").into(),
                data: None,
            })
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
    if let Some(next) = &hit.continues {
        out.push_str(&format!("\n  Continues: kuka://docs/{next}"));
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
             documentation (AMR fleet manuals, technical notes, safety, \
             electrical/mechanical print PDFs, and video demonstrations). \
             Recommended workflow for ANY KUKA question: (1) call search_docs \
             for text manuals and technical notes; (2) call list_media for media \
             assets like videos, electrical print schematics, mechanical drawings \
             (topics: electrical, mechanical, safety, vision, etc.); (3) if \
             search_docs excerpts do not fully answer, retry search_docs or \
             read the kuka://docs/{name} resource shown in the hit. Hits may \
             also list Diagrams: kuka://images/{name} resources. Do not \
             browse the resource list to hunt for answers, and never fall back \
             to reading source files outside these tools. list_docs shows \
             text documents by type. get_media fetches specific media details. \
             After re-extracting documentation, call reload_docs to rebuild \
             the index. ping confirms the server is alive."
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

        // Media items registered in the media registry (prints/schematics only; videos are metadata-only via list_media/get_media)
        {
            let registry = self.media_registry.read().unwrap();
            for item in registry.items() {
                if item.folder == "kuka-movies" || item.media_type == "video" {
                    continue;
                }
                let mut raw = RawResource::new(item.uri.clone(), item.id.clone())
                    .with_title(format!("{} [{}]", item.title, item.media_type))
                    .with_mime_type(item.mime_type.clone());
                if let Some(desc) = &item.description {
                    raw = raw.with_description(desc.clone());
                }
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

        // Media resources: kuka://media/...
        if let Some(relative_path) = uri.strip_prefix("kuka://media/") {
            let contents = self.read_media_resource_contents(relative_path, uri)?;
            return Ok(ReadResourceResult::new(vec![contents]));
        }

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

        let content = self.read_doc_resource_content(stem, uri)?;

        Ok(ReadResourceResult::new(vec![
            ResourceContents::text(content, uri.clone()).with_mime_type("text/markdown"),
        ]))
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

    let workspace_dir = knowledge_dir
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    let media_registry = MediaRegistry::load(&workspace_dir).unwrap_or_default();

    tracing::info!(
        "Starting KUKA MCP server: indexed {} document(s), {} unique term(s), {} media item(s) in {:.1?} (knowledge dir: {})",
        index.doc_count(),
        index.term_count(),
        media_registry.items().len(),
        started.elapsed(),
        knowledge_dir.display()
    );

    let server = KukaServer::new(knowledge_dir, workspace_dir, index, media_registry);
    match args.http {
        None => {
            // Attach the server to stdin/stdout and block until the client disconnects.
            let service = server.serve(stdio()).await?;
            service.waiting().await?;
        }
        Some(addr) => serve_http(addr, server, args.allowed_host).await?,
    }
    Ok(())
}

async fn serve_http(
    addr: String,
    server: KukaServer,
    extra_allowed_hosts: Vec<String>,
) -> Result<()> {
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

    let mut allowed_hosts = StreamableHttpServerConfig::default().allowed_hosts;
    allowed_hosts.extend(extra_allowed_hosts);
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default().with_allowed_hosts(allowed_hosts),
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

    #[test]
    fn args_default_to_no_extra_allowed_hosts() {
        let args = Args::try_parse_from(["mcp-server"]).unwrap();
        assert!(args.allowed_host.is_empty());
    }

    #[test]
    fn args_accept_repeated_allowed_host() {
        let args = Args::try_parse_from([
            "mcp-server",
            "--allowed-host",
            "kuka-mcp.example.dev",
            "--allowed-host",
            "kuka-mcp.example.dev:8443",
        ])
        .unwrap();
        assert_eq!(
            args.allowed_host,
            vec!["kuka-mcp.example.dev", "kuka-mcp.example.dev:8443"]
        );
    }

    #[test]
    fn extra_allowed_hosts_are_added_to_loopback_defaults() {
        let extra = vec!["kuka-mcp.example.dev".to_string()];
        let mut allowed_hosts = StreamableHttpServerConfig::default().allowed_hosts;
        allowed_hosts.extend(extra);
        let config = StreamableHttpServerConfig::default().with_allowed_hosts(allowed_hosts);
        assert!(config.allowed_hosts.contains(&"localhost".to_string()));
        assert!(config.allowed_hosts.contains(&"127.0.0.1".to_string()));
        assert!(config.allowed_hosts.contains(&"::1".to_string()));
        assert!(
            config
                .allowed_hosts
                .contains(&"kuka-mcp.example.dev".to_string())
        );
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
        let media_registry = MediaRegistry::load(temp_dir.path()).unwrap_or_default();
        KukaServer::new(
            temp_dir.path().to_path_buf(),
            temp_dir.path().to_path_buf(),
            index,
            media_registry,
        )
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

    fn write_chunk(dir: &std::path::Path, stem: &str, title: &str, pages: &str, body: &str) {
        let doc = format!(
            "---\ntype: manual\ntitle: {title}\nresource: kuka-docs/fleet.pdf\nparent: fleet-manual\npages: {pages}\n---\n\n{body}"
        );
        fs::write(dir.join(format!("{stem}.md")), doc).unwrap();
    }

    #[test]
    fn search_tool_shows_continues_line() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        write_chunk(
            temp_dir.path(),
            "fleet-manual-p001-008",
            "Fleet Manual (pages 1-8)",
            "1-8",
            "Opening uniquealpha chunk text.",
        );
        write_chunk(
            temp_dir.path(),
            "fleet-manual-p009-015",
            "Fleet Manual (pages 9-15)",
            "9-15",
            "Final uniquebeta chunk text.",
        );
        let server = server_over(&temp_dir);

        let first = server
            .search_docs(Parameters(SearchInput {
                query: "uniquealpha".to_string(),
            }))
            .unwrap();
        let first_text = result_text(&first);
        assert!(first_text.contains("Continues: kuka://docs/fleet-manual-p009-015"));

        let second = server
            .search_docs(Parameters(SearchInput {
                query: "uniquebeta".to_string(),
            }))
            .unwrap();
        let second_text = result_text(&second);
        assert!(!second_text.contains("Continues:"));
    }

    #[test]
    fn read_resource_appends_continuation_trailer() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        write_chunk(
            temp_dir.path(),
            "fleet-manual-p001-008",
            "Fleet Manual (pages 1-8)",
            "1-8",
            "Opening chunk body.",
        );
        write_chunk(
            temp_dir.path(),
            "fleet-manual-p009-015",
            "Fleet Manual (pages 9-15)",
            "9-15",
            "Final chunk body.",
        );
        let server = server_over(&temp_dir);

        let first = server
            .read_doc_resource_content("fleet-manual-p001-008", "kuka://docs/fleet-manual-p001-008")
            .unwrap();
        assert!(first.contains("[This section continues in kuka://docs/fleet-manual-p009-015]"));

        let second = server
            .read_doc_resource_content("fleet-manual-p009-015", "kuka://docs/fleet-manual-p009-015")
            .unwrap();
        assert!(second.contains("Final chunk body."));
        assert!(!second.contains("This section continues"));
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
        let media_registry = MediaRegistry::load(&bundle_dir).unwrap_or_default();
        let server = KukaServer::new(
            bundle_dir.clone(),
            bundle_dir.clone(),
            index,
            media_registry,
        );

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

    #[test]
    fn list_media_and_get_media_tools_work() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let movies_dir = temp_dir.path().join("kuka-movies");
        fs::create_dir_all(&movies_dir).unwrap();

        let json = r#"[
            {
                "id": "test-vid-1",
                "title": "Safety Setup Video",
                "type": "video",
                "topic": "safety",
                "keywords": ["safety", "zones"],
                "mimeType": "video/mp4",
                "folder": "kuka-movies",
                "filename": "safety.mp4",
                "uri": "kuka://media/kuka-movies/safety.mp4",
                "description": "Safety zones overview"
            }
        ]"#;
        fs::write(movies_dir.join("index.json"), json).unwrap();

        let dummy_doc = "\
---
type: technical-note
title: Dummy
resource: kuka-docs/d.pdf
---

Dummy text.";
        fs::write(temp_dir.path().join("dummy.md"), dummy_doc).unwrap();

        let index = Index::build(temp_dir.path()).unwrap();
        let media_registry = MediaRegistry::load(temp_dir.path()).unwrap();
        let server = KukaServer::new(
            temp_dir.path().to_path_buf(),
            temp_dir.path().to_path_buf(),
            index,
            media_registry,
        );

        let list_res = server
            .list_media(Parameters(ListMediaInput {
                topic: Some("safety".to_string()),
                keyword: None,
                media_type: None,
            }))
            .unwrap();
        let list_txt = result_text(&list_res);
        assert!(list_txt.contains("Safety Setup Video"));
        assert!(list_txt.contains("kuka://media/kuka-movies/safety.mp4"));

        let get_res = server
            .get_media(Parameters(GetMediaInput {
                id: Some("test-vid-1".to_string()),
                keyword: None,
            }))
            .unwrap();
        let get_txt = result_text(&get_res);
        assert!(get_txt.contains("Media Item: Safety Setup Video"));
        assert!(get_txt.contains("safety.mp4"));
    }

    #[test]
    fn read_resource_serves_media_blobs() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let prints_dir = temp_dir.path().join("kuka-prints");
        fs::create_dir_all(&prints_dir).unwrap();

        let json = r#"[
            {
                "id": "test-print-1",
                "title": "Test Print",
                "type": "print",
                "topic": "electrical",
                "keywords": ["test"],
                "mimeType": "application/pdf",
                "folder": "kuka-prints",
                "filename": "test_print.pdf",
                "uri": "kuka://media/kuka-prints/test_print.pdf"
            }
        ]"#;
        fs::write(prints_dir.join("index.json"), json).unwrap();
        fs::write(
            prints_dir.join("test_print.pdf"),
            b"%PDF-1.4 dummy pdf bytes",
        )
        .unwrap();

        let dummy_doc = "\
---
type: technical-note
title: Dummy
resource: kuka-docs/d.pdf
---

Dummy text.";
        fs::write(temp_dir.path().join("dummy.md"), dummy_doc).unwrap();

        let index = Index::build(temp_dir.path()).unwrap();
        let media_registry = MediaRegistry::load(temp_dir.path()).unwrap();
        let server = KukaServer::new(
            temp_dir.path().to_path_buf(),
            temp_dir.path().to_path_buf(),
            index,
            media_registry,
        );

        let uri = "kuka://media/kuka-prints/test_print.pdf";
        let relative_path = "kuka-prints/test_print.pdf";
        let contents = server
            .read_media_resource_contents(relative_path, uri)
            .unwrap();

        let val = serde_json::to_value(&contents).unwrap();
        assert_eq!(val["mimeType"], "application/pdf");
        assert_eq!(val["uri"], "kuka://media/kuka-prints/test_print.pdf");

        use base64::Engine as _;
        let blob_str = val["blob"].as_str().unwrap();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(blob_str)
            .unwrap();
        assert_eq!(decoded, b"%PDF-1.4 dummy pdf bytes");
    }

    #[test]
    fn video_resources_return_descriptive_text_payload() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let movies_dir = temp_dir.path().join("kuka-movies");
        fs::create_dir_all(&movies_dir).unwrap();

        let json = r#"[
            {
                "id": "test-video-1",
                "title": "Test Video",
                "type": "video",
                "topic": "safety",
                "keywords": ["test"],
                "mimeType": "video/mp4",
                "folder": "kuka-movies",
                "filename": "test_video.mp4",
                "uri": "kuka://media/kuka-movies/test_video.mp4"
            }
        ]"#;
        fs::write(movies_dir.join("index.json"), json).unwrap();
        fs::write(movies_dir.join("test_video.mp4"), b"dummy video bytes").unwrap();

        let index = Index::build(temp_dir.path()).unwrap();
        let media_registry = MediaRegistry::load(temp_dir.path()).unwrap();
        let server = KukaServer::new(
            temp_dir.path().to_path_buf(),
            temp_dir.path().to_path_buf(),
            index,
            media_registry,
        );

        let contents = server
            .read_media_resource_contents(
                "kuka-movies/test_video.mp4",
                "kuka://media/kuka-movies/test_video.mp4",
            )
            .unwrap();

        let val = serde_json::to_value(&contents).unwrap();
        let text = val["text"].as_str().unwrap();
        assert_eq!(val["mimeType"], "text/plain");
        assert!(text.contains("Video binary content is not served over MCP"));
        assert!(text.contains("Test Video"));
    }

    #[test]
    fn nonexistent_video_resource_returns_not_found_error() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let index = Index::build(temp_dir.path()).unwrap();
        let media_registry = MediaRegistry::load(temp_dir.path()).unwrap();
        let server = KukaServer::new(
            temp_dir.path().to_path_buf(),
            temp_dir.path().to_path_buf(),
            index,
            media_registry,
        );

        let err = server
            .read_media_resource_contents(
                "kuka-movies/nonexistent.mov",
                "kuka://media/kuka-movies/nonexistent.mov",
            )
            .unwrap_err();
        assert!(err.message.contains("Media resource not found"));
    }
}
