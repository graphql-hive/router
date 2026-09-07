---
hive-router: patch
---

# Close quiet upstream subscriptions after client disconnects

The router now closes an upstream subscription as soon as its final downstream client disconnects, even when the upstream emits no further events.

Closes https://github.com/graphql-hive/router/issues/1494
