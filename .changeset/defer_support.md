---
hive-router: minor
---

# `@defer` support

The router can now deliver parts of a response later with `@defer`. Turn it on with:

```yaml
defer:
  enabled: true
```

- The primary part goes out as soon as its data is there. Deferred fields get fetches of their own, which start as soon as their inputs are in, often before the primary part is sent, and never hold it back. That holds for fields from a subgraph the primary part already calls too: a deferred fragment on an entity goes back to that subgraph through the type's key, and a deferred root field of a query gets a root fetch of its own, so a slow resolver in a deferred fragment doesn't slow down the primary part. A type without a usable key is the exception: its deferred fields are fetched with the primary part, and only their delivery waits.
- Responses go out as `multipart/mixed`, SSE or WebSocket messages. The format is `incrementalSpec=v0.2` (`pending`, `incremental`, `completed`). A client that asks for `multipart/mixed;deferSpec=20220824`, like Apollo Client 3, gets the older format.
- `@defer(if: false)`, or an `if` variable that is `false`, gives a regular response. An `if` variable that isn't sent counts as `true`, the argument's default.
- Every `@defer` without a label is its own deferred part, and it goes out as soon as its own data is in, so a fast one doesn't wait for a slow sibling. A nested `@defer` goes out after the part it's nested in. Fetches of different deferred parts are never batched into one request. An operation with an active `@defer` from a client that only accepts `application/json` gets a `406` with `DEFER_TRANSPORT_NOT_SUPPORTED`.
- `@defer` isn't allowed on the root selection set of a mutation, and its labels are static strings that don't repeat in the document, like graphql-js 17 checks them. Subscriptions can't use `@defer` at all, even with `if: false`, they get a `DEFER_IN_SUBSCRIPTION` validation error.
- Fields that authorization rejects come out `null` in deferred parts too. Authorization runs before anything is fetched, so its errors go out with the primary part. A rejected non-null field in a deferred fragment fails that part only, the data around it stays.
- An exposed query plan shows deferred work in a `Defer` node: `Primary`, then a `Deferred` block per `@defer` with its label, path, the fetches it waits for (`depends`) and the fields it delivers (`{ reviews { body } }:`). Fetch ids are shown inside `Defer` nodes, so `depends` can point at them.
- A deferred fragment leaves out the fields that already went out for the same object, in the primary part or a part it's nested in, so they aren't sent twice.
- A `null` from a deferred fragment can't take away data another part delivers. When a non-null field fails in a deferred fragment and the `null` would land on a field the primary part or another fragment sends too, the fragment fails instead: it's `completed` with its errors and no data, whichever part goes out first. A `null` on a field only that fragment has goes out with it as usual, and so does a `null` that's in the data. A field counts as sent by another part only when that part has it for this request: its `@skip`/`@include` pass, the object is of the type it selects it on, and its own fragment didn't fail.
- Requests with `@defer` aren't deduplicated.
- With `traffic_shaping.entity_reuse` on, deferred fetches copy entities the response already has, like any other. A fetch never waits for a fetch of another deferred part just to copy from it, so the primary part isn't held back by a deferred one.
- The usage report, the "graphql request completed" log line, the request summary's error count and the HTTP server metrics come after the last part, or when the client goes away, with the errors of every part and the time until the last one. `graphql.response.status` is `error` when any part had errors.
- A response with `@defer` is always `Cache-Control: no-store`, whatever the subgraphs or plugins say. Its headers go out with the primary part, before the deferred fetches answer.
- `traffic_shaping.router.request_timeout` covers every part over HTTP. Past it nothing else starts or goes out, and the last part ends the fragments the client was told about with the timeout error and `hasNext: false`.
- Over WebSocket, `complete` cancels an operation with `@defer` like a subscription: deferred fetches that haven't started never do.
- `limits.max_defers` caps how many `@defer`s one operation can have, `@defer(if:)` included. A fragment's `@defer`s count once for every place it's spread. It defaults to 4. Going over it fails validation with `MAX_DEFERS_EXCEEDED`. Set it to `null` to turn it off.
- One plan covers every value of `@defer(if:)`: the fragment is planned both deferred and not, each way behind a variable the router sets, and the fetches of the other way are skipped. Subgraph queries carry those variables (`$_d0`, `$_d1`, ...) in `@skip`/`@include`, and an exposed plan shows both ways.
- A client that goes away stops the deferred work: fetches that haven't started never do.
- Plugin hooks run for `@defer` operations. `on_execute` end callbacks run once, before the primary part, and the new `has_next` field on their payload is `true` then. `on_graphql_error` runs for every part. The status code it sets for errors of the primary part is the response's status, like for a regular response. Later parts can't change it, the response head already went out. A plugin that answers from `on_execute` gets a regular response, like without `@defer`.

`InlineFragmentSelection` in `query_planner::ast` has a new `defer` field, so code that builds it with a struct literal needs `defer: None`.

Two fixes that apply to every operation:

- A client that only accepts `multipart/mixed` or `text/event-stream` gets a regular response as a single part, with the same status code it would have as JSON, instead of a `415` after the operation ran. A client that only accepts the multipart subscription protocol (`subscriptionSpec`) still gets `UNSUPPORTED_CONTENT_TYPE`.
- Conditional fragments at the root of a subgraph query go out without a type condition (`... @include(if: $x) { ... }`), so subgraphs that name their root type something other than `Query` accept them.
