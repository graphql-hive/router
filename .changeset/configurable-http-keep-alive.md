---
hive-router: minor
---

Allow overriding the HTTP server keep-alive timeout

Adds a new `traffic_shaping.router.keep_alive` configuration option (and `ROUTER_HTTP_KEEP_ALIVE` environment variable) to control how long the HTTP server waits for a follow-up request on an idle keep-alive connection before closing it.

This was previously fixed at ntex's 5 second default, with no Hive Router setting to change it. Behind a reverse proxy whose idle timeout is longer than 5 seconds, the router would close the socket first. The proxy then reused a connection the server had already dropped.

```yaml
traffic_shaping:
  router:
    keep_alive: 80s
```

When the router sits behind a reverse proxy, set this above that proxy's idle timeout so the proxy closes first.
