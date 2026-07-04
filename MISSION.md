# Mission: Build a KUKA Robot Knowledge MCP Server in Rust

## Why
Learn Rust through a real, meaningful project: a locally-running MCP server that lets AI agents like Claude query a library of KUKA AMR robot documentation PDFs. The immediate goal is personal productivity — getting accurate, fast answers about KUKA robots from an AI assistant grounded in official docs. The longer-term goal is to ship this as a product for external users.

## Success looks like
- A working MCP server written in Rust that runs locally via stdio
- Claude (or another AI agent) can query it and return accurate answers about KUKA robots sourced from the PDF library
- Comfortable enough with Rust to extend, refactor, and eventually productionize the server
- Understanding of MCP well enough to design the Tools and Resources the server exposes

## Constraints
- Starting from tutorial-level Rust (basic syntax and ownership, no async experience)
- Very little prior MCP knowledge
- PDFs are the source of truth — parametric AI knowledge about KUKA robots is not sufficient
- Personal use first; external shipping is a future phase

## Out of scope (for now)
- Cloud deployment or remote HTTP transport (stdio first)
- Multi-user authentication or access control
- Non-KUKA robot topics
- Building a frontend or chat UI
