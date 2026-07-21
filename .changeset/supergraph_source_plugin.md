---
hive-router-config: minor
---

# Add `supergraph.source: plugin` for deployments where plugins are the only source of supergraphs

This source creates no loader and has no configured fallback. Readiness, GraphQL requests, and WebSocket upgrades return service unavailable until the request's plugin selects a usable supergraph.

HTTP readiness checks invoke plugin's `on_http_request`, in order for the readiness check to pass - a supergraph must be selected even during that readiness request.

See `plugin_examples/feature_flags` for `supergraph.source: plugin` with all variants selected by the plugin.
