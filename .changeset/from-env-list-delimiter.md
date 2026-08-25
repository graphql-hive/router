---
hive-router: patch
---

`from_env` now supports list/array fields using `,` as the delimiter

A `from_env` placeholder whose `default` is a list is now resolved by splitting the environment
variable's value on `,` into one item per entry, instead of being kept as a single literal string
that would then fail to deserialize (or silently become a one-item list).

```yaml
cors:
  policies:
    - origins:
        from_env: CORS_ALLOWED_ORIGINS
        default:
          - http://localhost:3000
          - http://localhost:4000
```
