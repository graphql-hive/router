---
hive-router: patch
node-addon: patch
---

# Query planner errors are reported instead of hidden

A few places in the query planner turned an error into a wrong answer or a panic. They now return the error, so a plan that can't be made says why. Plans don't change.
