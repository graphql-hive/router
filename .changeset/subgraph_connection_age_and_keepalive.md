---
hive-router: patch
---

# Replace old subgraph connections, and detect dead ones

Three new settings under `traffic_shaping.all` control how long subgraph connections live:

```yaml
traffic_shaping:
  all:
    max_connection_age: 30s
    tcp_keepalive: 30s
    http2_keep_alive_interval: 10s
```

- `max_connection_age` (off by default). Under steady traffic a connection never goes idle, so `pool_idle_timeout` never closes it, and it keeps going to the same subgraph instance. Behind a Kubernetes Service, that keeps traffic on a failing pod, and a restarted pod gets almost none. With this set, a connection is retired once it's about this old (up to 10% earlier, so connections don't all reconnect at once). The age is checked when a request picks up a connection, so the request that finds it past its age still goes over it; the next request gets a new connection, which resolves DNS again. Requests already on the old connection finish normally.
- `tcp_keepalive` (30s by default). TCP keepalive probes start after this much silence on a connection and repeat at the same interval. After 3 unanswered probes the connection is closed, so a subgraph host that disappeared is noticed in about 2 minutes. It must be at least `1s` and is counted in whole seconds, so fractions are dropped (`1500ms` acts as `1s`). Set it to `null` to turn it off. Before, subgraph connections had no TCP keepalive.
- `http2_keep_alive_interval` (off by default). The router sends HTTP/2 PING frames at this interval and closes a connection whose ping isn't answered within 20 seconds. The subgraph process itself has to answer, so this also catches one that is hung. It only applies to HTTP/2 connections.

`max_connection_age` and `http2_keep_alive_interval` don't accept `0s`.
