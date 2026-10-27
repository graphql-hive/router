---
graphql-tools: minor
---

# Parse and format a schema one definition at a time

`parse_schema_definitions` parses a schema lazily, one definition per iteration, and `Definition::format_into` appends a definition to an existing buffer, the same way `Document::format` would print it. Together they let you process a huge schema without its whole AST in memory.
