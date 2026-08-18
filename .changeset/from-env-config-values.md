---
hive-router-config: minor
hive-router: patch
hive-router-internal: patch
hive-router-plan-executor: patch
---

# Support `{ from_env: "VAR" }` for any primitive config value

Any primitive field in the router config (strings, numbers, booleans, and the existing "either/or" fields like retry toggles or single-or-multiple lists) can now be set from an environment variable instead of a literal value:

```yaml
http:
  port:
    from_env: PORT
```

If the referenced environment variable is not set, the field falls back to its default (or fails validation as usual for required fields), and a warning is logged once the router's logger has started up.

You may also add an inline fallback can also be given with `default`, which is used instead of the field's own default when the environment variable is unset:

```yaml
http:
  port:
    from_env: PORT
    default: 4000
```
