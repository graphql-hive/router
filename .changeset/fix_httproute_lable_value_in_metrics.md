---
hive-router-internal: patch
hive-router: patch
hive-router-plan-executor: patch
---

# Fix `http.route` lable value in metrics

The `http.route` label value in metrics was set to the effective route path, instead of the route template.

In setups with `http.graphql_endpoint` is used, or when persisted documents are used over HTTP paths, this led to high cardinality in metrics.

Thanks [praguevara](https://github.com/praguevara) for contributing.
