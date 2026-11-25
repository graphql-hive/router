---
hive-router: patch
---

# Turn off `max_connections_per_host` by setting it to `0`

`traffic_shaping.max_connections_per_host` caps how many requests the router sends at once to one subgraph host, and defaults to `100`. There was no way to turn the cap off: setting it to `0` left no slots at all, so every subgraph request waited until it failed with `SUBGRAPH_REQUEST_SLOT_TIMEOUT`.

Now `0` removes the cap. Requests to a subgraph never wait for a slot, so they don't show up in the `hive.router.subgraph.requests.pending` and `hive.router.subgraph.request.slot_wait.duration` metrics. The pool of idle connections to each host, which normally keeps up to `max_connections_per_host` of them, has no size limit either; `pool_idle_timeout` still closes the ones that stay unused.

```yaml
traffic_shaping:
  max_connections_per_host: 0
```

Closes https://github.com/graphql-hive/router/issues/1959
