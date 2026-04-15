---
node-addon: patch
---

`Subscription` node's `primary` is `FetchNode` instead of `PlanNode` now, but the types were not compatible.
This change updates the type of `Subscription.primary` to be `FetchNode` instead of `PlanNode`.