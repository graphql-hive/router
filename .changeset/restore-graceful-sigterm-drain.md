---
hive-router: patch
---

Restore graceful HTTP server shutdown after the ntex-server 3.11 upgrade.

ntex-server 3.11 made graceful signal handling opt-in, while
`ntex::web::HttpServer` does not expose the new setting. This caused
`SIGTERM` to force-drop workers immediately and bypass the configured
`http.shutdown_timeout`. Pin `ntex-server` to 3.10.4 so SIGTERM drains
in-flight requests until ntex provides a supported opt-in on the public
HTTP server API.
