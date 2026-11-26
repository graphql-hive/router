---
hive-router: patch
node-addon: patch
---

# A `@requires` call waits for the fetch that returns its key

When the planner drops a fetch for a required field because another fetch already returns it, the `@requires` call that needed it keeps waiting for the fetch that returns its key. Before, it could lose that wait and end up in one request with that fetch, sending its representations without the key.
