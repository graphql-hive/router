---
hive-router-config: patch
hive-router: patch
hive-router-plan-executor: patch
hive-router-internal: patch
---

# Subgraph Error Masking

Mask subgraph errors before they reach clients, preventing internal details from leaking.

Masking is **enabled by default**: subgraph error messages are replaced with `"Unexpected error"`. It runs last in the pipeline, so metrics, tracing, and logging still see the original error.

Configure it under `error_masking`:

```yaml
error_masking:
  redacted_error_message: "Unexpected error"
  all:
    error_message: true
    extensions:
      mode: allow # allow | deny
      keys:
        - code
  subgraphs:
    products:
      enabled: false
```

- `error_message` toggles message redaction; `extensions` redacts extension keys via an `allow`/`deny` list.
- `subgraphs.<name>` overrides `all` per subgraph, inheriting any field it doesn't set.
- Set `DISABLE_SUBGRAPH_ERROR_MASKING=true` to disable message masking without editing the config.

[Documentation](http://the-guild.dev/graphql/hive/docs/router/security/error-masking)

Fixes https://github.com/graphql-hive/router/issues/1194
