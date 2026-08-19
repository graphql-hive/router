---
graphql-tools: patch
---

# Improve GraphQL Parser performance

Replace the `combine`-based document parser with a hand-written parser. No public API changes.

| Fixture | Before | After | Speedup |
|---|---|---|---|
| `minimal` | 1.08 µs | 184 ns | 5.9× |
| `inline_fragment` | 2.08 µs | 476 ns | 4.4× |
| `directive_args` | 2.19 µs | 568 ns | 3.9× |
| `query_vars` | 1.44 µs | 323 ns | 4.5× |
| `kitchen-sink` | 19.62 µs | 4.76 µs | 4.1× |
