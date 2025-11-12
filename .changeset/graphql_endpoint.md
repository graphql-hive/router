---
router: patch
config: patch
---

# `graphql_endpoint` Configuration and `.request.path_params` in VRL

- Adds support for configuring the GraphQL endpoint path via the `graphql_endpoint` configuration option.

So you can have dynamic path params that can be used with VRL expressions.

`path_params` are also added to `.request` context in VRL for more dynamic configurations.

```yaml
http:
  graphql_endpoint: /graphql/{document_id}
persisted_documents:
  enabled: true
  spec:
    expression: .request.path_params.document_id
```

[Learn more about the `graphql_endpoint` configuration option in the documentation.](https://the-guild.dev/graphql/hive/docs/router/configuration/graphql_endpoint)

[Learn more about the `.request.path_params` configuration option in the documentation.](https://the-guild.dev/graphql/hive/docs/router/configuration/expressions#request)