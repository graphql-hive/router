---
router: patch
config: patch
---

# New Query Complexity Configuration w/ Max Depth Rule

We have introduced a new configuration module for query complexity in the Hive Router. This includes a new validation rule to enforce maximum query depth, helping to prevent overly complex queries that could impact performance.

By default, it is disabled, but you can enable and configure it in your router configuration as follows:

```yaml
query_complexity:
  max_depth:
    n: 10  # Set the maximum allowed depth for queries
```

This configuration allows you to set a maximum depth for incoming GraphQL queries, enhancing the robustness of your API by mitigating the risk of deep-nested queries.