---
hive-router: patch
hive-router-config: patch
---

# New Operation Complexity Option: Max Aliases

We've introduced a new configuration option, `max_aliases` that allows you to limit the number of aliases in the incoming GraphQL operations. This helps to prevent overly complex queries that could impact performance, or any potential DOS attack or heap overflow via excessive aliases.

```yaml
limits:
  max_aliases:
    n: 3  # Set the maximum number of aliases allowed in a query
```
