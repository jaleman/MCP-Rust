# MCP Resources Introduced (Lesson 24)

MCP Resources were introduced as the second MCP primitive alongside Tools. Key points established:

- Resources are data endpoints addressed by URI; Tools are callable actions that take parameters
- REST analogy: Resources ≈ GET, Tools ≈ POST
- URI scheme chosen: `kuka://docs/{stem}` where stem = filename without `.md`
- `Resource` is a type alias for `Annotated<RawResource>` in rmcp
- Two methods override `ServerHandler`: `list_resources` and `read_resource`
- Both are async and take `RequestContext<RoleServer>` (unused, but required by the trait)
- `ErrorCode::RESOURCE_NOT_FOUND` (−32002) is the correct error for unknown URIs
- `ResourceContents::text(content, uri)` — content first, URI second (non-obvious order)
- Capabilities must advertise `.enable_resources()` or Claude never sends `resources/list`

**Implications**: Can reference "resources vs tools" distinction without re-explaining. Can
reference `kuka://docs/{stem}` URI scheme as established. Do not re-explain the REST analogy.
Subscription-based resource updates (live changes) and resource templates are unexplored — introduce explicitly when needed.
