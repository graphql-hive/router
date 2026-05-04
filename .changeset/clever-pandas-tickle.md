---
hive-router-internal: minor
hive-router-plan-executor: minor
hive-router: minor
---

## Improve HTTP server request OTel tracing with client and peer network attributes.

The `http.server` span now includes:
- `client.address` and `client.port` from a configurable request header
- `network.peer.address` and `network.peer.port` from the address of the incoming connection

```yaml
telemetry:
  client_identification:
    # Default - use socket peer only
    ip_header: null
    
    # Header name - use the left-most valid IP from the header
    ip_header: x-forwarded-for
    
    # Trusted proxies - only trust the header when the socket peer is trusted
    ip_header:
      name: x-forwarded-for
      trusted_proxies:
        - 10.0.0.0/8
        - 192.168.0.0/16
```

In trusted proxies scenario, the Router scans the configured header from right to left, skips trusted proxy IP ranges, and records the first non-trusted IP as `client.address`.
If no valid client IP can be resolved, the Router falls back to the socket peer address.
