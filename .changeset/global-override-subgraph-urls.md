---
hive-router-config: major
hive-router-plan-executor: minor
hive-router: minor
---

# Global `override_subgraph_urls.all` and path parameters in override expressions

Closes [#985](https://github.com/graphql-hive/router/issues/985).

## Breaking: new shape for `override_subgraph_urls`

`override_subgraph_urls` is no longer a flat map keyed by subgraph name. Per-subgraph overrides now live under a `subgraphs` key, alongside a new optional `all` override. The redundant `url` wrapper has also been removed: each entry under `subgraphs.<name>` is now either a static URL string or an object with an `expression` field directly.

```yaml
# Before
override_subgraph_urls:
  accounts:
    url: "https://accounts.example.com/graphql"
  products:
    url:
      expression: |
        if .request.headers."x-region" == "us-east" {
          "https://products-us-east.example.com/graphql"
        } else {
          .default
        }

# After
override_subgraph_urls:
  subgraphs:
    accounts: "https://accounts.example.com/graphql"
    products:
      expression: |
        if .request.headers."x-region" == "us-east" {
          "https://products-us-east.example.com/graphql"
        } else {
          .default
        }
```

## New: `override_subgraph_urls.all`

A single override applied to every subgraph that does not have its own per-subgraph override. Useful when the override logic is the same (or only depends on the subgraph name) for every subgraph in the supergraph. Per-subgraph entries under `subgraphs.<name>` always take precedence over `all`.

```yaml
override_subgraph_urls:
  all:
    expression: |
      if .subgraph.name == "products" && .request.headers."x-region" == "us-east" {
        "https://products-us-east.example.com/graphql"
      } else {
        .default
      }
```

The expression has access to:

- `.request`: the incoming HTTP request (headers, query parameters, the parsed GraphQL operation, and `url_matches`, see below).
- `.default`: the original subgraph URL from the supergraph SDL.
- `.subgraph.name`: the name of the subgraph the URL is being resolved for.

## New: path parameters under `.request.url_matches`

`http.graphql_endpoint` already supports ntex's native `{name}` segments. Any path parameters captured from the configured pattern are now exposed to override expressions (and any other VRL expression that has access to the request) under `.request.url_matches`.

```yaml
http:
  graphql_endpoint: /{tenant}/graphql
override_subgraph_urls:
  all:
    expression: |
      tenant = string!(.request.url_matches.tenant)
      replace(string!(.default), "/api/", "/api/" + tenant + "/")
```

A request to `/acme/graphql` resolves `tenant` to `"acme"` before the expression runs.
