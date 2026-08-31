---
hive-router: patch
---

Upgrade `ntex` internal crates to latest

Bumps `ntex-server` to `3.11.2` (and its `ntex-net`/`ntex-rt`/`ntex-error` siblings to matching latest versions), dropping the pins introduced in #1445.

`ntex-server` `3.11.0`/`3.11.1` made graceful signal handling opt-in and defaulted it off, with no way for `ntex::web::HttpServer` to opt back in — a real `SIGTERM` force-dropped in-flight requests and ignored `http.shutdown_timeout` entirely. `3.11.2` restores the previous behavior, so the router no longer needs to hold these crates back at `3.10.4`/`3.14.1`/`3.16.1`.
