---
hive-router: minor
---

# `@defer` support

The router can now send parts of a response later with `@defer`. It's off by default:

```yaml
defer:
  enabled: true
```

The primary part goes out as soon as its data is there. Each deferred fragment follows once its own data is in, so a fast one doesn't wait for a slow one.

**Transport**

- Responses go out as `multipart/mixed`, SSE or WebSocket messages.
- The format is `incrementalSpec=v0.2` (`pending`, `incremental`, `completed`). A client that asks for `multipart/mixed;deferSpec=20220824`, like Apollo Client 3, gets the older format.
- A client that only accepts `application/json` gets a `406` with `DEFER_TRANSPORT_NOT_SUPPORTED`, before anything is fetched.
- `@defer(if: false)`, or an `if` variable that's `false`, gives a regular response. An `if` variable that isn't sent counts as `true`.

**Fetching**

- Deferred fields get fetches of their own. They start as soon as their inputs are in and never hold back the primary part.
- That works for fields of the subgraph the primary part already calls too, when the fragment is on an entity: the router goes back to that subgraph through the type's `@key`. On a type that isn't an entity there, with no `@key` or only `@key(resolvable: false)`, those fields are fetched with the primary part, and only their delivery waits.
- A nested `@defer` goes out after the fragment it's nested in.
- A client that goes away, or a WebSocket `complete`, stops the deferred work: fetches that haven't started never do.

**Data and errors**

- A deferred fragment leaves out the fields that already went out for the same object.
- A `null` from a deferred fragment can't take away data another part delivers. When a non-null field fails and the `null` would land on a field the primary part or another fragment sends too, the fragment fails instead: it's `completed` with its errors and no data.
- Fields that authorization rejects come out `null` in deferred parts too. The authorization errors go out with the primary part.

**Limits**

- `limits.max_defers` caps the `@defer`s in one operation, 4 by default. A fragment's `@defer`s count once for every place it's spread. Going over it fails validation with `MAX_DEFERS_EXCEEDED`, `null` turns the limit off.
- `traffic_shaping.router.request_timeout` covers every part over HTTP. Past it, the fragments still pending end with the timeout error.
- No `@defer` in subscriptions (`DEFER_IN_SUBSCRIPTION`) or on the root selection set of a mutation. Labels are static strings that don't repeat in the document.
- A response with `@defer` is always `Cache-Control: no-store`. Its headers go out with the primary part, so headers from subgraphs that only deferred fragments call aren't passed on.
- Requests with `@defer` aren't deduplicated.

**Plugins, telemetry and query plans**

- `on_execute` end callbacks run once, before the primary part, with the new `has_next: true` on their payload. `on_graphql_error` runs for every part, but only the primary part's errors can change the status code.
- The usage report, the "graphql request completed" log line and the HTTP server metrics come after the last part, or when the client goes away, with the errors of every part. `http.server.response.body.size` counts every part.
- The `http.server`, `graphql.operation` and `graphql.execute` spans stay open until the last part, and the deferred subgraph calls are in the same trace, under `graphql.execute`. `http.server` gets `hive.defer.end_reason`: `completed`, or `client_disconnect` when the client left first.
- An exposed query plan shows deferred work in a `Defer` node, with a `Deferred` block per `@defer`. With `@defer(if:)` it shows the fragment both ways, deferred and not, each behind a variable the router sets (`$_d0`, `$_d1`, ...).

**For code that uses the router as a library**

`InlineFragmentSelection` in `query_planner::ast` has a new `defer` field, so code that builds it with a struct literal needs `defer: None`.

**Two fixes for every operation**

- A client that only accepts `multipart/mixed` or `text/event-stream` gets a regular response as a single part, with the status code it would have as JSON, instead of a `415` after the operation ran. A client that only accepts the multipart subscription protocol (`subscriptionSpec`) still gets `UNSUPPORTED_CONTENT_TYPE`.
- Conditional fragments at the root of a subgraph query go out without a type condition (`... @include(if: $x) { ... }`), so subgraphs that name their root type something other than `Query` accept them.
