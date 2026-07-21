---
hive-router-plan-executor: minor
hive-router: patch
---

# Select a supergraph in the `on_http_request` plugin hook

Add `OnHttpRequestHookPayload::set_supergraph`, allowing a plugin to select a stable `Arc<Supergraph>` for an HTTP request or WebSocket upgrade. The selected supergraph is used consistently for validation, introspection, normalization, planning, demand control, execution, coprocessors, usage reporting, and request deduplication.

`Supergraph` contains schema-derived state only and can be built with `Supergraph::from_sdl` or `Supergraph::from_document`. Router-specific state, including subgraph executors and schema-aware caches, remains owned by the router. The router builds configured runtimes eagerly and plugin-selected runtimes lazily, reusing them through a bounded FIFO cache.

Plugins own the lifetime of their supergraphs. Dropping the last `Arc<Supergraph>` retires that supergraph: ordinary in-flight requests finish from their snapshots, active subscriptions close with the schema-reload error, and the router removes any cached plugin runtime in the background. Runtime eviction (when the internal bounded FIFO cache of supergraph runtimes evicts) does not retire a supergraph - if reused later, the router will rebuild the internal supergraph runtime.

See `plugin_examples/replace_schema` for overriding a configured default and `plugin_examples/feature_flags` for plugin-only supergraph selection.
