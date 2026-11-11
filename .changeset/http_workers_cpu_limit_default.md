---
hive-router: minor
---

# HTTP server workers follow the container's CPU limit by default

The default `http.workers: cpuset` sizes the HTTP server worker pool with ntex's own count: one worker per CPU the process is allowed to run on, capped by the container's CPU limit (the cgroup CPU quota), rounded down, with a minimum of `1`. In Kubernetes, a pod with a `1500m` CPU limit runs a single worker instead of one per CPU of the node.

`container` is the same as `cpuset` and stays accepted, so existing configurations keep working. Set an explicit number of workers to run more workers than the CPU limit allows.

```yaml
http:
  workers: 4
```
