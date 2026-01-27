---
hive-router-config: patch
hive-router: patch
---

# Support multiple endpoints in Hive CDN Supergraph config

In order to support a Secondary CDN endpoint for better reliability, the Hive CDN Supergraph configuration has been updated to allow specifying either a single endpoint or multiple endpoints.
This change enables users to provide a list of CDN endpoints, enhancing the robustness of their supergraph setup.

[Learn more about it in the relevant Hive Console documentation here](https://the-guild.dev/graphql/hive/docs/schema-registry/high-availability-resilence).

```diff
supergraph:
    source: hive
-    endpoint: https://cdn-primary.example.com/supergraph
+    endpoint:
+       - https://cdn-primary.example.com/supergraph
+       - https://cdn-secondary.example.com/supergraph
```