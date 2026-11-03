---
hive-router: minor
---

# Upgrade to ntex 4

The router runs on ntex 4.5.0 instead of ntex 3. Users of the prebuilt binary or the Docker image do not need to change anything: the configuration, the HTTP and websocket APIs and VRL expressions are unchanged.

Custom plugins and custom router binaries built on the `hive-router` crate need the migration steps below. Code that uses ntex APIs directly through `hive_router::ntex`, such as building responses, writing ntex services or using ntex test helpers, follows the upstream [ntex 3 to 4 migration guide](https://github.com/ntex-rs/ntex/blob/main/ntex/migration_3to4.md).

## Rust 1.97 or newer is required

ntex 4 requires Rust 1.97, so every crate that depends on `hive-router` has to build with at least that version.

## BREAKING: Request URLs use `ntex::url::Url`

ntex 4 represents request URLs with `ntex::url::Url` instead of `http::Uri`, and `ntex::http::Uri` no longer exists. The following APIs expose the new type:

- `RouterHttpRequest::uri` and `RouterHttpRequest::match_info` (`Path<ntex::url::Url>`)
- `ClientRequestDetails::url`, `MutableClientRequestDetails::url` and `ClientRequestDetailsView::url()`
- the coprocessor `MutableRequestState::uri`
- the `url` argument of `execute_planned_request`
- `HttpServerSpanRequest::uri()`
- `OnHttpRequestHookPayload::router_http_request.uri()`

`Url::path()` and `Url::query()` return wrapper types that do not deref to `str`, so string methods need `.as_str()`. Comparing them with a string literal and formatting them with `Display` work without changes.

```diff
-use hive_router::http::Uri;
+use hive_router::ntex::url::Url;

-if payload.router_http_request.uri.path().starts_with("/admin") {
+if payload.router_http_request.uri.path().as_str().starts_with("/admin") {

-let query: Option<&str> = payload.router_http_request.uri.query();
+let query: Option<&str> = payload.router_http_request.uri.query().map(|q| q.as_str());
```

`RouterHttpRequest::path` and `RouterHttpRequest::query_string` are still plain `&str` and avoid the new type entirely.

```diff
-let path = payload.router_http_request.uri.path();
-let query = payload.router_http_request.uri.query().unwrap_or("");
+let path = payload.router_http_request.path;
+let query = payload.router_http_request.query_string;
```

`host()`, `port_u16()` and `scheme_str()` keep their signatures. `port()` returns a different `Port` type, so use `port_u16()` when only the number is needed.

```diff
-let port = details.url().port().map(|p| p.as_u16());
+let port = details.url().port_u16();
```

APIs that still take an `http::Uri`, such as `reqwest` or `hyper`, need an explicit conversion. It can fail and it drops the fragment.

```diff
-let uri: http::Uri = details.url().clone();
+let uri = http::Uri::try_from(details.url()).unwrap_or_default();
```

Tests that build a `RouterHttpRequest` by hand create the URL with the new type.

```diff
-let uri: http::Uri = "http://example.com/graphql".parse().unwrap();
-let path: Path<http::Uri> = Path::new(uri.clone());
+let uri: ntex::url::Url = "http://example.com/graphql".parse().unwrap();
+let path: Path<ntex::url::Url> = Path::new(uri.clone());
```

## BREAKING: `on_http_request` payload type

`OnHttpRequestHookPayload::router_http_request` is a `WebRequest<()>` instead of a `WebRequest<DefaultError>`. Plugins that name the type in their own functions have to update it.

```diff
-use hive_router::ntex::web::{DefaultError, WebRequest};
+use hive_router::ntex::web::WebRequest;

-fn fingerprint(req: &WebRequest<DefaultError>) -> u64 {
+fn fingerprint(req: &WebRequest<()>) -> u64 {
```

## `on_end` callbacks on websocket connections run when the socket closes

The websocket handler serves the whole connection inside the request, so a websocket upgrade request finishes only when the socket closes. For such a request, the `on_end` callback returned from `on_http_request` runs after the connection closes, and changes it makes to the response are never sent, because the handshake was already answered.

Plugins that measure request durations, keep in-flight counters or set response headers on the websocket path see different results. Logic that has to run at the handshake belongs in `on_http_request` itself.

## Timers never fire early

ntex 4 timers fire at or after their deadline, up to one 16ms tick late, while ntex 3 timers could fire early. Periodic work such as supergraph polling runs at the configured interval or slightly later, never sooner.

## Persisted documents file watching follows symlinks

The persisted documents file watcher reloads the manifest when its path goes through a symlinked directory, such as the temp directory on macOS. File system events on macOS report resolved paths, so the watcher matches events against both the configured and the canonicalized manifest path.
