---
hive-router-query-planner: minor
node-addon: minor
---

# Query Plan Subscriptions Node

The query planner now emits a `Subscription` node when planning a subscription operation. The `Subscription` node contains a `primary` fetch that is sent to the subgraph owning the subscription field.
