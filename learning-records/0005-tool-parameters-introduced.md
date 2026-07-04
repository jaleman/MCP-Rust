# Tool Parameters Introduced (Lesson 5)

serde and schemars were introduced as the two-crate pattern for rmcp tool parameters. Covered at teaching level: `#[derive(Deserialize, JsonSchema)]` on a parameter struct, `#[tool(param)]` on the function argument, and the fact that `///` doc comments become JSON Schema descriptions.

**Implications**: Can reference "the serde derive" and "the input schema" without re-explaining them. Has not yet encountered optional parameters (`Option<T>` fields), multi-field structs, or `#[serde(rename)]` — introduce explicitly when needed. format! was used for the first time here.
