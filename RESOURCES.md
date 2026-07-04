# KUKA MCP Server in Rust — Resources

## Knowledge

### MCP (Model Context Protocol)

- [MCP Official Documentation](https://modelcontextprotocol.io/docs)
  The canonical spec and conceptual guides. Start with "Introduction" and "Core architecture." Use for: understanding the protocol, the three primitives (Tools, Resources, Prompts), and transport options.

- [Build an MCP Server — Official Guide](https://modelcontextprotocol.io/docs/develop/build-server)
  Step-by-step walkthrough of building a server. Use for: understanding the lifecycle of a server, how tools are registered, how requests flow.

- [rmcp crate docs (docs.rs)](https://docs.rs/rmcp)
  API reference for the official Rust MCP SDK. Use for: finding the right types and traits when writing server code.

- [modelcontextprotocol/rust-sdk on GitHub](https://github.com/modelcontextprotocol/rust-sdk)
  Source code + examples for the official Rust SDK. Use for: working examples, understanding idiomatic usage, checking issues and migration guides.

- [Building MCP Servers in Rust — Rustify Guide (2026)](https://rustify.rs/articles/rust-for-mcp-model-context-protocol-servers-2026)
  Practical walkthrough of MCP server construction with rmcp. Use for: getting oriented on patterns before writing code.

- [Build a Weather MCP Server with Rust — Paul Yu](https://paulyu.dev/article/rust-mcp-server-weather-tutorial/)
  Complete worked example of a Rust MCP server that calls an external API. Use for: seeing a full end-to-end server before building ours.

### Rust

- [The Rust Programming Language (The Book)](https://doc.rust-lang.org/book/)
  The definitive Rust learning resource. Use for: any Rust language question — ownership, traits, error handling, modules. Chapters 1–11 are the foundation; return to it constantly.

- [Tokio Async Runtime](https://tokio.rs)
  The async runtime used by rmcp. Use for: understanding async/await in Rust, spawning tasks, working with futures.

- [Tokio Tutorial](https://tokio.rs/tokio/tutorial)
  Hands-on introduction to async Rust with Tokio. Use for: learning async Rust before writing the server's async code.

### PDF Processing

- [pdf-extract crate (GitHub)](https://github.com/jrmuizel/pdf-extract)
  Pure-Rust library for extracting text content from PDFs. Use for: pulling text out of KUKA PDF files for indexing.

- [lopdf crate (GitHub)](https://github.com/J-F-Liu/lopdf)
  Lower-level PDF manipulation library. Use for: cases where pdf-extract doesn't handle a PDF correctly and you need finer control.

- [How to Extract Text from PDF in Rust — Ahmad Rosid](https://ahmadrosid.com/blog/extract-text-from-pdf-in-rust)
  Short, practical walkthrough of PDF text extraction in Rust. Use for: seeing a working extraction example before writing the ingestion pipeline.

## Wisdom (Communities)

- [The Rust Programming Language Discord](https://discord.gg/rust-lang)
  Official Rust community Discord. Use for: asking questions about Rust language features, async patterns, and crate choices.

- [r/rust on Reddit](https://reddit.com/r/rust)
  Large, high-quality Rust community. Use for: broader questions, crate recommendations, sharing progress.

- [MCP Discord (via modelcontextprotocol.io)](https://modelcontextprotocol.io)
  Community around MCP development. Use for: MCP-specific questions, seeing what others are building.

## Gaps

- No official KUKA AMR developer documentation found online — knowledge will come exclusively from the PDFs you provide.
- Need to evaluate whether `pdf-extract` handles KUKA's specific PDF formatting (may need testing once PDFs are available).
