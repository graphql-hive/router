---
hive-router-config: patch
hive-router: patch
--- 

Support multiple endpoints for Hive Console CDN source for Supergraph.
So you can pass endpoints separated by comma in the env var `HIVE_CDN_ENDPOINT` and it will be split and set as an array of endpoints in the config. This allows for better load balancing and failover when using Hive Console CDN as the source for the Supergraph.

```
HIVE_CDN_ENDPOINT=https://cdn.graphql-hive.com/***,https://cdn-mirror.graphql-hive.com/***
```

[Learn more about CDN mirrors](https://the-guild.dev/graphql/hive/docs/schema-registry/high-availability-cdn#cdn-mirrors)
