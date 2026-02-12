---
hive-router-internal: patch
hive-router: patch
hive-router-plan-executor: patch
---

Make `hive.inflight.key` span attribute unique per inflight group, for better identification of the leader and joiners in a distributed system.
