---
hive-router: minor
---

# Contribute inbound dedupe partitions from the validation hook

The `add_inbound_dedupe_partition` setter is now also available on the
`on_graphql_validation` hook's payload, alongside the existing setters on
`on_http_request` and `on_graphql_params`.

Closes https://github.com/graphql-hive/router/issues/1507
