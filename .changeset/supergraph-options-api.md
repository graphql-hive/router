---
hive-router-plan-executor: major
hive-router: patch
---

# Bind graph-specific configuration to `Supergraph`

`Supergraph::from_sdl` and `Supergraph::from_document` now accept `SupergraphOptions` instead of `QueryPlannerOptions`. The immutable options snapshot includes planner, executor, subgraph subscription, persisted-document, error-masking, and Hive target settings.
