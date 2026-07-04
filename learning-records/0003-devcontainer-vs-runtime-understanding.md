# Understood: Dev Container vs Runtime Container Are Separate Concerns

Initially confused about how a dev container (for writing code) and a runtime container (for running the MCP server) would interact. After explanation, understood they are completely independent: the dev container is a coding environment that goes away when development stops; a runtime container is optional packaging for the compiled binary. Chose to use a dev container for development; runtime containerisation is deferred (already out-of-scope in MISSION.md).

**Implications**: No need to explain the dev/runtime container distinction again. Can reference the dev container as "where you write code" without qualification.
