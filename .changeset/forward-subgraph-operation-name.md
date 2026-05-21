---
hive-router: patch
hive-router-config: patch
hive-router-plan-executor: patch
---

# Forward operationName to subgraphs

Subgraph HTTP and HTTP callback requests now include the planner-assigned `operationName` in the JSON request body by default. Added `traffic_shaping.all.strip_operation_name` and per-subgraph `traffic_shaping.subgraphs.<name>.strip_operation_name` options for deployments that need to preserve the previous omission.

Global opt-out:

```yaml
traffic_shaping:
  all:
    strip_operation_name: true
```

Per-subgraph opt-out:

```yaml
traffic_shaping:
  subgraphs:
    products:
      strip_operation_name: true
```
