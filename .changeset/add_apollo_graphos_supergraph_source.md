---
hive-router-config: minor
hive-router: minor
hive-router-internal: patch
hive-router-plan-executor: patch
---

# Add `apollo_graphos` supergraph source

Adds a new `apollo_graphos` supergraph source that fetches the supergraph schema from Apollo GraphOS's managed federation Uplink, for routers migrating from Apollo Router/Gateway without needing a separate schema-delivery pipeline.

Configure it with `graph_ref` and `key` (or the `APOLLO_GRAPH_REF`/`APOLLO_KEY` environment variables), and optionally `endpoint` (defaults to Apollo's GCP and AWS Uplink endpoints, tried in order), `timeout` and `accept_invalid_certs`.

Closes https://github.com/graphql-hive/router/issues/505
