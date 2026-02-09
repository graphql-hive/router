---
hive-router: minor
hive-router-config: minor
---

# New configuration option to set a timeout for the router

This update introduces a new configuration option that allows users to set a timeout for the router. This timeout will help prevent long-running requests from consuming resources indefinitely, improving the overall performance and reliability of the router. Users can now specify a timeout duration in their configuration files, and the router will automatically terminate any requests that exceed this duration.

By default, the timeout is set to 60 seconds;

```yaml
traffic_shaping:
    router:
        request_timeout: 60s # Human readable duration format (e.g., "30s", "1m", "2h")
```