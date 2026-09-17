---
hive-router: minor
---

Named supergraphs, surfaced in logs, traces and metrics

Every `Supergraph` now carries a human-readable `name`. The supergraph loaded from the router configuration (`supergraph.source`) is named `default`. Plugins that construct their own supergraphs - for example to serve a different schema variant per tenant or feature flag - give each one a name, and that name is what the router reports when describing which supergraph handled a request.

### Plugin API

`Supergraph::from_sdl` and `Supergraph::from_document` take the name as a new, required first argument:

```rust
// before
let variant = Supergraph::from_document(document, options)?;

// after
let variant = Supergraph::from_document("my-supergraph", document, options)?;
```

#### Logging

The request summary line (`router::request` target) reports the name of the supergraph that processed the request:

- **Added** `supergraph_name` - the selected supergraph's name, e.g. `"default"` or the name a plugin chose. It is absent when the request was rejected before a supergraph was selected.
- **Removed** `supergraph_identifier` - the internal numeric id previously printed in its place. It was not stable across restarts or reloads and was never meaningful to operators.

Example (JSON format):

```json
{"level":"INFO","target":"router::request","operation_type":"query","status_code":200,"supergraph_name":"default","duration_ms":12,...}
```

#### Traces

The `graphql.operation` span gains the attribute:

- `hive.supergraph.name` - the selected supergraph's name.

It is recorded as soon as the supergraph is selected, for both HTTP and WebSocket (`graphql-ws`) operations.

Only the operation span carries it; child spans (parse, validate, plan, subgraph calls) are unchanged and can be attributed to a supergraph through their parent.

#### Metrics

A `supergraph.name` label is added to:

- `http.server.request.duration`
- `hive.router.graphql.errors_total`

Both now break down by supergraph, so a deployment serving several schema variants from one router can chart latency and error rates per variant.

For `http.server.request.duration` the label is omitted on requests that failed before a supergraph was selected (e.g. malformed HTTP requests).

For `hive.router.graphql.errors_total` it is omitted for errors raised before selection.

Closes https://github.com/graphql-hive/router/issues/1565
