---
hive-router: minor
---

# Warm up the plan cache before switching to a new supergraph

Every new supergraph starts with an empty plan cache, so the first requests after a schema publish wait on the query planner. With `cache.warmup` turned on, the router plans known operations against the new supergraph while the previous one keeps serving, and only switches over once that's done.

```yaml
cache:
  warmup:
    enabled: true # off by default
    timeout: 30s # switch over with whatever got planned by then
```

Operations come from, in this order:

- every operation the previous supergraph served,
- plugins, through the new `on_cache_warmup` hook, which can add, drop or reorder the list.

`order` decides which operations of the previous supergraph go first, which matters once `timeout` cuts warmup short. It's `slowest_to_plan` by default, or `most_used`, or `recently_used`. It can also be an expression that scores each operation, where `null` leaves it out:

```yaml
cache:
  warmup:
    enabled: true
    order:
      expression: |
        if .operation.type == "mutation" { null } else { .operation.plan_duration_ms * .operation.hits }
```

The expression gets `.operation.type`, `.operation.hits` and `.operation.plan_duration_ms`, plus `.operation.last_used_ms_ago`, which can be `null`. The counts start from zero with every new supergraph.

Plans are built for requests with no progressive override labels. Requests with active labels, and operations that authorization or plugins trim, plan on their first request like before.

Two new metrics show how it went:

- `hive.router.plan_cache.warmup.operations_total`, by `source` (`previous_supergraph`, `plugin`) and `result` (`warmed`, `failed`, `skipped`, `timed_out`)
- `hive.router.plan_cache.warmup.duration`, by `result` (`completed`, `timed_out`)
