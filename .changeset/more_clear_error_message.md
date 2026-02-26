---
hive-router: patch
hive-router-config: patch
---

In case of errors when the supergraph path is unable to be resolved, the error message now includes the path that failed to resolve and the starting path used for resolution. This provides more context for debugging issues related to supergraph configuration.

```diff
- Failed to canonicalize path "supergraph.graphql": No such file or directory (os error 2)
+ Failed to canonicalize path "supergraph.graphql" on "/path/to/config": No such file or directory (os error 2)
```