---
hive-router-config: major
hive-router-plan-executor: minor
hive-router: minor
hive-router-internal: patch
---

# BREAKING: `override_subgraph_urls.subgraphs` and global `all`

In `override_subgraph_urls` the per-subgraph overrides now live under a `subgraphs` field, alongside a new optional `all` override.

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
  all:
    url:
      expression: |
        if .subgraph.name == "reviews" {
          "https://reviews.example.com/graphql"
        } else {
          .default
        }
```

A single override under `override_subgraph_urls.all.url` is now applied to every subgraph that does not have its own per-subgraph override. This is useful when the override logic is the same for all subgraphs or depends on the current subgraph name.

The expression has access to:

- `.request`: the incoming HTTP request
- `.default`: the original subgraph URL from the supergraph SDL
- `.subgraph.name`: the name of the subgraph the URL is being resolved for

Per-subgraph entries under `subgraphs.<name>` always take precedence over `all`.
