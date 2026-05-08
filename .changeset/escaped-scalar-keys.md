---
hive-router-plan-executor: patch
hive-router: patch
---

#  Fix subgraph response deserialization for custom scalar object

Values whose JSON keys contain escaped characters such as `\t` are now deserialized correctly.
