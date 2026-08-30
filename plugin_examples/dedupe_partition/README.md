# Dedupe Partition Example

This example demonstrates `add_inbound_dedupe_partition`, which plugins can use to contribute an
extra component to the router's inbound (query/subscription) dedupe fingerprint, without having
to reimplement fingerprinting themselves.

The underlying partition data lives in the request context, but the setter is only exposed on the
hook payloads that run early enough to affect the fingerprint — `OnHttpRequestHookPayload` and
`OnGraphQLParamsStartHookPayload` — instead of on the generic context. Calling it from a later
hook (e.g. `on_execute`) is a compile error rather than a silent no-op, since by then the dedupe
claim has already happened.

The problem it solves: the built-in `traffic_shaping.router.dedupe.headers` allowlist can only
include or exclude whole headers. A `Cookie` header usually carries more than identity (CSRF
tokens, analytics IDs, ...), so including it in the dedupe key defeats deduplication for
practically every request, while excluding it collapses requests from different users into one
shared response.

This plugin extracts a JWT from a configured cookie in `on_graphql_params` (before the router
computes the dedupe fingerprint), validates it, and calls
`payload.add_inbound_dedupe_partition(hash_of_sub)`:

- Two requests with the same validated `sub` claim share a dedupe partition — same behavior as
  today's dedupe, just scoped to that user.
- Two requests with different `sub` claims never share a partition, and so never share a
  response.
- A missing or invalid token leaves the context untouched, so the request falls into the default,
  shared "anonymous" partition. This assumes invalid tokens are otherwise treated as anonymous by
  the rest of the stack — if your deployment instead rejects invalid tokens while allowing
  anonymous access, a rejected request could join an anonymous leader's response, and you'd want
  to partition invalid tokens by a random value instead of leaving the context untouched.

Since identity now comes from this plugin, the config below sets `headers: none` on the built-in
policy — otherwise the router would still hash the raw `Cookie` header on top of the plugin's
partition, defeating the point.

See `src/plugin.rs` for the implementation and `src/test.rs` for the dedupe assertions.
