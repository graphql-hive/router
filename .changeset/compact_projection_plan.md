---
hive-router: patch
node-addon: patch
---

# Use Less Memory for Response Projection Plans

The Router now stores response projection plans in a more compact form.

This reduces the memory used when the Router handles many different queries. In our memory test, each new query used about 20% less memory.
