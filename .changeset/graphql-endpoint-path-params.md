---
hive-router-plan-executor: minor
hive-router: minor
hive-router-config: minor
---

# Path parameters from `http.graphql_endpoint`

Any path parameters captured from the configured pattern are now exposed:

- in expressions as `.request.path_params`
- in plugins through the existing `RouterHttpRequest.match_info`

```yaml
http:
  graphql_endpoint: /{tenant}/graphql
override_subgraph_urls:
  all:
    url:
      expression: |
        tenant = string!(.request.path_params.tenant)
        replace(string!(.default), "/api/", "/api/" + tenant + "/")
```

A request to `/acme/graphql` resolves `tenant` to `"acme"` before the expression runs.
