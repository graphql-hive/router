---
hive-router: minor
hive-router-config: minor
---

# Allow overriding number of HTTP server workers

Adds a new `http.workers` configuration option (and `WORKERS` environment variable) to control the number of HTTP server worker threads.

By default, the router spawns one worker per physical CPU core. In containerized environments such as Kubernetes the number of physical cores reported by the OS is often higher than the CPU limit assigned to the container, which leads to oversubscribed worker threads. Set `http.workers` (or `WORKERS`) to match the container's CPU limit to avoid this.

```yaml
http:
  workers: 4
```
