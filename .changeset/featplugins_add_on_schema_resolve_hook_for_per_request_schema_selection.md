---
hive-router: minor
hive-router-plan-executor: minor
---

# feat(plugins): add on_schema_resolve hook for per-request schema selection

#1123 by @martinw-ct

Adds an `on_schema_resolve` hook to the `RouterPlugin` trait, letting a plugin
choose — per request, before the pipeline runs — which schema the request is
validated and planned against. This lets a single router serve more than one
schema (e.g. a different supergraph per tenant) without forking the entrypoint,
with the selection logic (path, header, auth, …) left to the plugin.

A plugin selects a schema by inserting a `RequestSchema` into the request's
`PluginContext`; the router runs the pipeline against it (and an optional
per-request shared state), falling back to the default app-state schema when none
is selected. The hook runs for queries, mutations, and subscriptions, and a
plugin may also short-circuit with a response.
