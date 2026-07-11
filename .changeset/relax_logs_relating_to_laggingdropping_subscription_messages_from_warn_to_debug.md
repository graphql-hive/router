---
hive-router: patch
hive-router-plan-executor: patch
---

# Log lagged and dropped subscription messages at `debug` level

Reduce expected slow-consumer noise by changing logs for lagged client messages and messages dropped from full subscription buffers from `warn` to `debug`. These events no longer imply subscription termination and can be monitored through the subscription metrics.
