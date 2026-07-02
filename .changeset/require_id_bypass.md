---
hive-router-plan-executor: minor
hive-router: minor
---

# Allow to bypass persisted document ID requirement

Add `hive::persisted_documents::skip_enforcement` request context flag. When set to `true`, the router skips the persisted document ID requirement. This allows requests with a full operation string to pass through even when `persisted_documents.require_id` is enabled.

The flag is writable from `OnHttpRequest` and `OnGraphqlParams` plugin hooks, and from the `router.request` coprocessor stage.
