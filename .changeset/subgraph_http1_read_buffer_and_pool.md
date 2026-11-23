---
hive-router: patch
---

# Smaller read buffers and an experimental HTTP/1 pool for subgraphs

The read buffer of an HTTP/1.1 connection to a subgraph grows to about 400 KB on big responses and stays that big while the connection is busy, so hundreds of busy connections hold a lot of memory. You can now cap it with `traffic_shaping.all.http1_max_read_buffer_size`, and per subgraph with `traffic_shaping.subgraphs.<name>.http1_max_read_buffer_size`. Bigger responses then arrive in more chunks, and the router still reads them into one buffer. Subgraph response headers have to fit in it. It has to be at least `8KiB`, and it's not set by default:

```yaml
traffic_shaping:
  all:
    http1_max_read_buffer_size: 64KiB
  subgraphs:
    reports:
      http1_max_read_buffer_size: 1MiB
```

A smaller value saves memory under load, a bigger one helps subgraphs that send very large responses. HTTP/2 connections don't use it.

`traffic_shaping.experimental_http1_pool` sends requests to plain `http://` subgraphs through the router's own HTTP/1 connection pool. It opens a new connection only when every connection of the pool is busy, so far fewer sockets stay open under load. It follows `http_client_mode`: one pool for all workers with `shared`, one per worker with `per_worker`. It uses `http1_max_read_buffer_size` like the regular client, and closes idle connections after `pool_idle_timeout`. `https://` subgraphs, `allow_only_http2` and HTTP subscriptions keep using the regular client. Off by default.
