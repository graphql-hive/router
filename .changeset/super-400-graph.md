---
hive-console-sdk: patch
hive-router: patch
---

Fixed: 4xx client errors are now properly treated as errors and trigger endpoint failover, instead of being returned as successful responses.
This ensures the CDN fallback mechanism works correctly when endpoints return client errors like 403 Forbidden or 404 Not Found.
