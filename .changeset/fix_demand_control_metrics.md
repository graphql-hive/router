---
hive-router: major
hive-router-internal: major
---

# Fix missing demand control metrics names

Previously added metrics names were missing the `hive.router.demand_control.` prefix. This has been fixed in this change. 

Here's a list of the metrics that have been fixed:

- `cost.estimated` -> `hive.router.demand_control.cost.estimated`
- `cost.actual` -> `hive.router.demand_control.cost.actual`
- `cost.delta` -> `hive.router.demand_control.cost.delta`
