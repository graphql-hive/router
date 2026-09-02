---
hive-router: patch
---

Fix: custom-plugin support for `@policy` decisions

Added support for making `@policy` decisions in custom-plugin `on_graphql_analysis` hooks. 

Plugin `on_graphql_analysis` hooks now run before authorization enforcement, the same way a coprocessor already did, so a plugin can decide `@policy` grants just like a coprocessor can.
