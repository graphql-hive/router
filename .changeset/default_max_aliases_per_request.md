---
hive-router: minor
---

# `max_aliases_per_request` defaults to 15

The number of aliased `_entities` fields in one subgraph request now defaults to 15, graphql-armor's own default for its max-aliases rule.

With no cap, a contextual fetch sends one aliased `_entities` field per distinct `@fromContext` value in the response, all in one request. That number comes from the data, not from the query, so a subgraph guarded by graphql-armor with default settings starts refusing requests the day a list grows past 15 distinct values, and nothing in a test environment with less varied data predicts it. Groups past the cap are sent in additional requests that run in parallel, and entity batches with more aliases than the cap are split the same way.

**This changes behaviour** for fetches that previously carried more than 15 aliases: they are now split into several parallel requests. To restore one request per fetch, raise the cap:

```yaml
traffic_shaping:
  all:
    max_aliases_per_request: 1000
```
