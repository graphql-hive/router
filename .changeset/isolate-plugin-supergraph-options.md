---
hive-router: minor
---

# Isolate plugin-selected supergraph configuration

Plugin-selected supergraphs no longer inherit graph-bound settings from the router's configured supergraph. Requests, WebSocket connections, persisted-document resolution, subgraph execution, usage reports, and Hive traces now use the options attached to the selected `Supergraph` snapshot.

Persisted-document reloaders and Hive usage agents now use separate background-task groups scoped to the selected supergraph runtime. Cancelling a runtime waits for any active Hive flush and explicitly flushes the remaining report buffer before removing its worker. Router shutdown also waits for these graceful background tasks to finish.

## Migration

Configured supergraphs require no YAML changes. The router continues deriving their graph-bound options from the existing configuration.

Plugins that construct supergraph variants must import `SupergraphOptions`, provide the graph-bound settings each variant needs, and retain the owner while it remains selectable:

```rust
use std::sync::Arc;

use hive_router::plugins::hooks::on_supergraph_load::{Supergraph, SupergraphOptions};

let mut options = SupergraphOptions::default();
options.traffic_shaping.all.forward_operation_name = true;
options.error_masking.redacted_error_message = "Variant error".to_string();
options.hive_target = Some("organization/project/variant".to_string());

let variant = Arc::new(Supergraph::from_sdl(sdl, options)?);
```

Move any existing `QueryPlannerOptions` into `SupergraphOptions::query_planner`. Also copy every graph-specific setting the variant previously inherited from router configuration, including subgraph traffic shaping, URL overrides, headers, override labels, demand control, subscription transports, error masking, persisted documents, and its Hive target. Omitted fields use `SupergraphOptions::default()` and no longer inherit values from the configured supergraph.
