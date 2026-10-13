---
hive-router: minor
---

# Read deadlines for request headers and body

A client could send the request headers and then send the body slowly, or never. The router kept its socket and buffers until `traffic_shaping.router.request_timeout`, with no limit on how many such requests it held, and enough of them got the router OOM-killed.

Two new options bound how long the router waits for an incoming request:

- `header_read_timeout`: time to receive the complete request headers. When exceeded, the connection is closed. Rounded up to whole seconds.
- `body_read_timeout`: time to receive the complete request body, from the moment the router starts reading it. When exceeded, the request is rejected with `408 Request Timeout` (`PAYLOAD_READ_TIMEOUT`) and the connection is closed.

Both default to `10s`, and `0s` disables them.

```yaml
traffic_shaping:
  router:
    header_read_timeout: 10s
    body_read_timeout: 10s
```

This also fixes how the existing timeouts behaved:

- The header deadline was `request_timeout` plus one second (61s by default), so raising `request_timeout` for slow operations also let clients take that long to send headers. It is now independent, and clients that take longer than `10s` to send headers are disconnected.
- A stalled body used up `request_timeout` and got `504 Gateway Timeout` (`GATEWAY_TIMEOUT`), which blamed the upstream for a slow client. It now gets a `408` once `body_read_timeout` runs out, as long as that is shorter than `request_timeout`.
- The coprocessor `router.request` stage with `include_body` reads the body before `request_timeout` starts, so that read had no deadline at all. `body_read_timeout` now applies to it.

`header_read_timeout` runs before `request_timeout` starts, so the two add up.

`body_read_timeout` runs within `request_timeout`, so it only takes effect when it is shorter, except in the coprocessor `router.request` stage.

If clients upload large bodies over slow connections, consider to raise `body_read_timeout`.

Closes https://github.com/graphql-hive/router/issues/1690
