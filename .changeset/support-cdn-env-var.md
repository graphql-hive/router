---
hive-router-config: patch
hive-router: patch
--- 

Support multiple endpoints for Hive Console CDN source for Supergraph.
So you can pass endpoints separated by comma in the env var `HIVE_CDN_ENDPOINT`, so that if one CDN endpoint is not available, the router can fallback to the next one in the list.

```
HIVE_CDN_ENDPOINT=https://cdn.graphql-hive.com/***,https://cdn-mirror.graphql-hive.com/***
```

[Learn more about CDN mirrors](https://the-guild.dev/graphql/hive/docs/schema-registry/high-availability-cdn#cdn-mirrors)
