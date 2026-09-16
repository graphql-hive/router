---
hive-console-sdk: minor
---

Improve the usage-report operation cache key

`OperationProcessor` now keys its cache with an xxh3 hash of the operation body and, when `process_variables` is enabled, the shape of the variables payload: variable names, object keys, nesting and list lengths, never values.
