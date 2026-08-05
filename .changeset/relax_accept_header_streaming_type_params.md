---
hive-router: patch
---

# Accept multipart streaming types with boundary parameters

Hive Router now accepts `boundary` parameters on `multipart/mixed` values in the `Accept` header. This prevents Apollo Client's default Accept header from returning `415 Unsupported Media Type` while leaving response boundary selection to the router.
