---
hive-router: patch
---

# Reliably reload a supergraph file mounted from a Kubernetes `ConfigMap`

Fixes intermittent `Failed to read supergraph file: No such file or directory` errors (and stale schemas) when the supergraph is loaded from a file mounted from a Kubernetes `ConfigMap` with polling enabled.

The file supergraph poller additionally retries transient `NotFound`/`ESTALE` errors that can occur if a read races with the atomic swap, and detects content changes by any modification-time difference (not only newer timestamps), so a rolled-back revision is still picked up.

Closes https://github.com/graphql-hive/router/issues/1516
