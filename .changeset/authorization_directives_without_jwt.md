---
hive-router: minor
---

# `@authenticated` and `@requiresScopes` are enforced when JWT is not enabled

When `jwt.enabled` was not `true`, the router skipped `@authenticated` and `@requiresScopes` entirely and served protected fields to anonymous callers, with HTTP 200 and no error. That was the case with no `jwt` block at all, and with a `jwt` block that configured providers but left `enabled` out. Nothing in the response or the logs said the directives were off.

Without JWT no request can authenticate, so every request is now treated as unauthenticated: `@authenticated` fields and `@requiresScopes` fields are refused, exactly as they are for an anonymous caller when JWT is enabled.

**This changes behaviour** for a deployment that serves a supergraph with these directives and has JWT disabled. If you rely on another layer for authorization and want the directives ignored, say so explicitly:

```yaml
authorization:
  directives:
    enabled: false
```
