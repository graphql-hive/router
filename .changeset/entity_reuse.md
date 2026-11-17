---
hive-router: patch
---

# Copy entities already in the response

An entity fetch often asks for entities an earlier fetch already put in the response. With entity reuse on, the fetch copies those into place and leaves them out of the request. When it has every entity, the subgraph isn't called at all. Off by default:

```yaml
traffic_shaping:
  entity_reuse: true
```

Or with the `ROUTER_TRAFFIC_SHAPING_ENTITY_REUSE=true` environment variable.

A fetch copies only from the fetches it waits for. It may also wait for an earlier fetch to the same subgraph that asked for the same fields, when that gives it something to copy. It never copies once the request has an error, in mutations, fields a subgraph only has through `@provides`, or entities whose representation has a missing or null field. Fetches with `@fromContext` arguments, input or output rewrites don't copy.

Closes #1730
