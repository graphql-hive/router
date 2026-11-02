---
hive-router: minor
---

# Size HTTP server workers to the container's CPU limit

`http.workers` (and `ROUTER_HTTP_WORKERS`) now accepts `cpuset` and `container`, in addition to an explicit number of workers.

- `cpuset` (default, unchanged behavior): one worker per logical CPU the process is allowed to run on (its CPU affinity mask, which follows the cgroup cpuset on Linux). CPU limits are not taken into account, so in Kubernetes this is usually the number of CPUs of the node.
- `container`: like `cpuset`, but also capped by the container's CPU limit (the cgroup CPU quota), rounded down, with a minimum of `1`. For example, a `1500m` CPU limit results in a single worker.
- A positive integer: an explicit number of workers, as before.

```yaml
http:
  workers: container
```

Closes https://github.com/graphql-hive/router/issues/1561
