---
hive-router: patch
---

# Raise the open files limit on start

On start, the router now raises its soft open files limit up to the hard limit. It logs the old and new limits, or a warning if the raise fails.
