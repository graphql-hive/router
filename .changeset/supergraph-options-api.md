---
hive-router-plan-executor: major
hive-router: patch
---

# Bind graph-specific configuration to `Supergraph`

`Supergraph::from_sdl` and `Supergraph::from_document` now accept `SupergraphOptions` instead of `QueryPlannerOptions`. The immutable options snapshot includes planner, executor, subgraph subscription, persisted-document, error-masking, and Hive target settings.

## Migration

Import `SupergraphOptions` with `Supergraph`:

```rust
use hive_router::plugins::hooks::on_supergraph_load::{Supergraph, SupergraphOptions};
```

Replace the query-planner options argument with a complete supergraph options value:

```diff
-use hive_router::plugins::hooks::on_supergraph_load::Supergraph;
+use hive_router::plugins::hooks::on_supergraph_load::{Supergraph, SupergraphOptions};
 use hive_router::query_planner::planner::QueryPlannerOptions;

 let query_planner = QueryPlannerOptions {
     experimental_abstract_type_folding: true,
 };
-let supergraph = Supergraph::from_sdl(sdl, query_planner)?;
+let supergraph = Supergraph::from_sdl(
+    sdl,
+    SupergraphOptions {
+        query_planner,
+        ..SupergraphOptions::default()
+    },
+)?;
```

Callers that used `Default::default()` can migrate directly:

```rust
use hive_router::plugins::hooks::on_supergraph_load::{Supergraph, SupergraphOptions};

let supergraph = Supergraph::from_sdl(sdl, SupergraphOptions::default())?;
```
