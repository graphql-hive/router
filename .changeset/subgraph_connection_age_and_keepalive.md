---
hive-router: patch
---

# Replace old subgraph connections, and detect dead ones

Three new settings under `traffic_shaping.all`:

```yaml
traffic_shaping:
  all:
    max_connection_age: 30s
    tcp_keepalive: 30s
    http2_keep_alive_interval: 10s
```

- `max_connection_age` (off by default) retires a connection at roughly this age, up to 10% earlier so they don't all reconnect at once. Busy connections never go idle, so without it they stay on the same subgraph instance; behind a Kubernetes Service that keeps traffic on a failing pod. The age is checked when a request on the connection gets its response: from then on the connection takes no new requests, and the next one opens a new connection, resolving DNS again.
- `tcp_keepalive` (30s by default, `null` turns it off) sends TCP keepalive probes after this much silence and closes the connection after 3 unanswered, so a vanished host is noticed in about 2 minutes. At least `1s`, in whole seconds. Subgraph connections had no TCP keepalive before.
- `http2_keep_alive_interval` (off by default) sends HTTP/2 pings and closes a connection whose ping isn't answered within 20s. The subgraph process has to answer, so this also catches a hung one.

`max_connection_age` and `http2_keep_alive_interval` don't accept `0s`.
