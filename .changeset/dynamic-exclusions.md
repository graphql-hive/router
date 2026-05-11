---
apollo-router-hive-fork: minor
---

# Dynamic Exclusions in Apollo-Router

As in Hive Router, Apollo Router used to support only operation name based exclusions. 

With the new dynamic exclusions feature available in the Hive fork of Apollo-Router, you can now specify custom logic to exclude requests from usage reporting.

```yaml
# legacy format
exclude:
  - ExcludedOp

# dynamic expression format
exclude:
  expression: '.request.operation.name == "ExcludedOp"'
```
