---
hive-router-plan-executor: minor
hive-router: minor
---

# Allow to bypass persisted document ID requirement

Add `hive::persisted_documents::skip_enforcement` request context flag. When set to `true`, the router skips the persisted document ID requirement. This allows requests with a full operation string to pass through even when `persisted_documents.require_id` is enabled.

The flag is writable from `OnHttpRequest` and `OnGraphqlParams` plugin hooks, and from the `router.request` coprocessor stage.

`persisted_documents.require_id` now accepts an expression in addition to a static boolean. The expression is evaluated per-request and supports request context (`.request.headers`, `.request.method`, `.request.url`) and the `env()` function for environment variables. When the expression returns `true`, the document ID requirement is enforced, when `false`, it is skipped for that request.

Example:

```yaml
persisted_documents:
  require_id:
    expression: |
      is_null(env("BYPASS_SECRET")) || .request.headers."x-bypass-require-id" != env("BYPASS_SECRET")
```
