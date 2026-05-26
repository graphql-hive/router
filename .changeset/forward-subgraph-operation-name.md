---
hive-router: patch
hive-router-config: patch
hive-router-plan-executor: patch
---

# Forward operationName to subgraphs

Subgraph HTTP and HTTP callback requests can now include a planner-assigned `operationName` in the JSON request body. Added the `traffic_shaping.all.forward_operation_name` and per-subgraph `traffic_shaping.subgraphs.<name>.forward_operation_name` options. The option defaults to `false`, so the previous omission remains the default behavior.

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
      forward_operation_name: true
```
