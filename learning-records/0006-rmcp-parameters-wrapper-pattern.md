# rmcp 1.8.0 Tool Parameter Pattern Corrected

The `#[tool(param)]` attribute syntax taught in Lesson 5's first draft does not exist in rmcp 1.8.0. The correct pattern uses a `Parameters<T>` wrapper type imported from `rmcp::handler::server::wrapper::Parameters`. Tool functions receive arguments via destructuring in the function signature:

```rust
fn search_docs(&self, Parameters(input): Parameters<SearchInput>) -> Result<CallToolResult, McpError>
```

`Parameters(input)` unwraps the wrapper and binds the inner struct to `input` directly. This was discovered through compiler errors and confirmed against the rmcp 1.8.0 source.

**Implications**: Any future tool with parameters uses this pattern. The `handler::server::wrapper::Parameters` import must be added to the `use rmcp::{ ... }` block. Do not reference `#[tool(param)]` — it does not compile.
