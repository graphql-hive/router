---
hive-router-plan-executor: minor
hive-router-query-planner: patch
hive-router-config: patch
hive-router: patch
node-addon: patch
hive-router-internal: patch
---

## Forward operation name to subgraphs

Added the `traffic_shaping.all.forward_operation_name` and `traffic_shaping.subgraphs.<name>.forward_operation_name` options. The option defaults to `false`.

The operation name is injected (opt-in) into the query document and the `operationName` JSON field, formatted as `<client_operation_name>__<fetch_step_id>`, when sending requests to subgraphs.

Global opt-in:

```yaml
traffic_shaping:
  all:
    forward_operation_name: true
```

Per-subgraph opt-in:

```yaml
traffic_shaping:
  subgraphs:
    products:
      # Overrides global setting for this subgraph
      forward_operation_name: true
```
