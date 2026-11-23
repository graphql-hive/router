---
hive-router: patch
---

# Smaller read buffers and an experimental HTTP/1 pool for subgraphs

The subgraph HTTP client caps its read buffer at 64 KB per HTTP/1.1 connection. Before, the buffer grew to about 400 KB on a connection that got big responses and kept that while the connection was busy, so hundreds of busy connections held a lot of memory. Bigger responses now arrive in more chunks, and the router still reads them into one buffer. Subgraph response headers have to fit in it.

You can change the cap with `traffic_shaping.all.http1_max_read_buffer_size`, and per subgraph with `traffic_shaping.subgraphs.<name>.http1_max_read_buffer_size`. It has to be at least `8KiB`:

```yaml
traffic_shaping:
  all:
    http1_max_read_buffer_size: 64KiB
  subgraphs:
    reports:
      http1_max_read_buffer_size: 1MiB
```

A smaller value saves memory under load, a bigger one helps subgraphs that send very large responses. HTTP/2 connections don't use it.

`traffic_shaping.experimental_http1_pool` sends requests to plain `http://` subgraphs through the router's own HTTP/1 connection pool. It opens a new connection only when every connection of the pool is busy, so far fewer sockets stay open under load. It follows `http_client_mode`: one pool for all workers with `shared`, one per worker with `per_worker`. `https://` subgraphs, `allow_only_http2` and HTTP subscriptions keep using the regular client. Off by default.
