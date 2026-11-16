---
hive-router: patch
---

# Less memory under bursts of subgraph traffic

The subgraph HTTP client caps its read buffer at 64 KB per connection. Before, the buffer grew to about 400 KB on a connection that got big responses and kept that while the connection was busy, so hundreds of busy connections held a lot of memory. Bigger responses now arrive in more chunks, and the router still reads them into one buffer. Subgraph response headers have to fit in it.

You can change the cap with `traffic_shaping.all.http1_max_read_buffer_size`, and per subgraph with `traffic_shaping.subgraphs.<name>.http1_max_read_buffer_size`:

```yaml
traffic_shaping:
  all:
    http1_max_read_buffer_size: 64KiB
  subgraphs:
    reports:
      http1_max_read_buffer_size: 1MiB
```

A smaller value saves memory under load, a bigger one helps subgraphs that send very large responses. HTTP/2 connections don't use it.

New connections that need a subgraph's name while a lookup for it is running now wait for that lookup and share its answer. Nothing is kept after that, so the next connection looks the name up again. Before, every new connection ran its own lookup on a separate thread, so a burst of new connections started hundreds of threads.

Together, under a burst from 50 to 500 concurrent clients, the router's peak memory dropped by about a third, with the same throughput and CPU use.
