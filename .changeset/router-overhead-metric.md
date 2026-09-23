---
hive-router: minor
---

# Router overhead metrics

Two new histograms report how much of a request's latency is the router's own work versus time spent waiting on external services:

- `hive.router.request.overhead.duration` - the request duration (`http.server.request.duration`) minus the time the request spent waiting on external services. This is the latency the router adds on top of your upstreams: parsing, validation, planning, response merging, plugins.
- `hive.router.request.external_wait.duration` - the time the request spent waiting on at least one external service.

The following are considered "external" and count as waiting:

- Persisted document fetches from the Hive CDN on a cache miss also count as waiting

- Subgraph calls 

- Co-processor calls

They are not recorded for streamed responses (subscriptions, incremental delivery), whose subgraph waits continue after the response headers are sent.

With explicit histogram buckets (the default), `hive.router.request.overhead.duration` uses fine-grained boundaries from 100µs to 1s instead of the shared `seconds` buckets, since router overhead is typically sub-millisecond.

Closes https://github.com/graphql-hive/router/issues/1579
