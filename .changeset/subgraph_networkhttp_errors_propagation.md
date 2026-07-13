---
hive-router-plan-executor: patch
hive-router: patch
---

# Subgraph network/HTTP errors propagation

Improves how the router reacts to failed or malformed subgraph HTTP responses. Instead of silently ignoring the HTTP status and content-type, the router now emits a GraphQL error for the failing fetch. The affected field is set to `null` and partial results from other subgraphs are preserved.

| Subgraph response | Router behavior |
| --- | --- |
| `2xx`, valid content-type, valid GraphQL body | passed through unchanged |
| `2xx`, valid content-type, valid JSON but no `data`/`errors` | `SUBREQUEST_MALFORMED_RESPONSE` |
| `2xx`, valid content-type, body is not valid JSON | `SUBREQUEST_MALFORMED_RESPONSE` |
| `2xx`, missing or non-JSON content-type (e.g. `text/html`) | `SUBREQUEST_MALFORMED_RESPONSE` |
| Non-2xx status | `SUBREQUEST_HTTP_ERROR` (with `extensions.http.status`) |
| Transport/connection failure (no HTTP response) | `SUBREQUEST_HTTP_ERROR` |

Fixes [#1229](https://github.com/graphql-hive/router/issues/1229)
