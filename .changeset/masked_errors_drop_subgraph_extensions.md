---
hive-router: patch
---

# Masked subgraph errors no longer forward the subgraph's own `extensions`

Error masking replaced the message with "Unexpected error" and passed the subgraph's `extensions` through untouched, so a stack trace, a database host or an SQL error reached the client under a masked message.

When a subgraph error's message is masked and no `extensions` rule is configured for it, only the keys the router adds itself are now kept: `code`, `service`, `affectedPath` and `http`. An explicit `extensions` allow or deny list, globally or per subgraph, still takes precedence, and turning message masking off still forwards everything.

```yaml
error_masking:
  all:
    extensions:
      mode: allow
      keys: ["code", "service", "traceId"]
```
