---
hive-router: patch
---

Restore graceful HTTP server shutdown after the ntex-server 3.11 upgrade.

ntex-server 3.11 made graceful signal handling opt-in and defaults it to false,
while `ntex::web::HttpServer` exposes no way to opt in. This caused `SIGTERM` to
force-drop workers immediately and bypass the configured `http.shutdown_timeout`.

Hold `ntex-server` at 3.10.4, together with `ntex-net` 3.14.1 and `ntex-rt`
3.16.1, so that `SIGTERM` drains in-flight requests again. All three move
together because ntex-rt 3.17 changed the `Signal` enum in a way 3.10.4 cannot
build against, and ntex-net 3.15 requires that newer ntex-rt.
