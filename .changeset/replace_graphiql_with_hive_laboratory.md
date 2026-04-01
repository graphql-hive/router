---
hive-router: patch
hive-router-config: patch
---

# Replace GraphiQL with Hive Laboratory

The Laboratory is Hive's powerful GraphQL playground that provides a comprehensive environment for exploring, testing, and experimenting with your GraphQL APIs. Whether you're developing new queries, debugging issues, or sharing operations with your team, the Laboratory offers all the tools you need.

Read more about Hive Laboratory in [the introduction blog post](https://the-guild.dev/graphql/hive/product-updates/2026-01-28-new-laboratory) or [the documentation](https://the-guild.dev/graphql/hive/docs/new-laboratory).

### Breaking Changes:

The top-level config option has been renamed.

```diff
- graphiql:
+ laboratory:
    enabled: true
```

So was the environment variable override.

```diff
- GRAPHIQL_ENABLED=true
+ LABORATORY_ENABLED=true
```
