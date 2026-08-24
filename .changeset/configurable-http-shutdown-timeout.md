---
hive-router: minor
---

Allow overriding the HTTP server graceful shutdown timeout

Adds a new `http.shutdown_timeout` configuration option (and `ROUTER_HTTP_SHUTDOWN_TIMEOUT` environment variable) to control how long the HTTP server waits for in-flight requests to complete after receiving `SIGTERM`, before remaining workers are force-dropped.

This was previously fixed at ntex's 30 second default, which is unrelated to `traffic_shaping.router.request_timeout`. A router configured to allow requests longer than 30 seconds would drop those requests mid-flight on every rolling deploy, even when the surrounding platform was willing to wait. The default stays at 30 seconds, so existing behaviour is unchanged.

```yaml
http:
  shutdown_timeout: 90s

traffic_shaping:
  router:
    request_timeout: 85s
```

Set it above `traffic_shaping.router.request_timeout` so the longest request the router accepts can still finish during a drain. In orchestrated environments the platform's own grace period (for example Kubernetes' `terminationGracePeriodSeconds`) must in turn exceed `http.shutdown_timeout`, otherwise the process is killed before the drain completes.
