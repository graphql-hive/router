---
hive-router: minor
hive-router-config: minor
hive-router-internal: patch
hive-router-plan-executor: patch
---

# External storage support (e.g S3)

[documentation](http://the-guild.dev/graphql/hive/docs/router/configuration/storages)

This release introduces a new top-level `storages` configuration and the first storage backend, s3, so the router can load external artifacts from object storage.

With this change, both the `supergraph` source and `persisted_documents` manifest can be resolved from a configured storage by reference. It also adds optional polling support so the router can reload updated content from storage without restarting.

Start by configuring the storage in your router config:

```yaml
storages: 
  my-s3: # this is the storage id 
    type: s3
    bucket: my-bucket
    region: eu-west-1
    # .. additional S3 configurations 
```

Then, you can use the storage id in your `supergraph` source:

```yaml
supergraph:
  source: storage
  storage_id: my-s3
  location: supergraphs/current.graphql
  poll_interval: 30s
```

Or, you can use the storage id in your `persisted_documents` manifest:

```yaml
persisted_documents:
  enabled: true
  require_id: true
  storage:
    type: storage
    storage_id: my-s3
    location: persisted/manifest.json
    poll_interval: 30s
```
