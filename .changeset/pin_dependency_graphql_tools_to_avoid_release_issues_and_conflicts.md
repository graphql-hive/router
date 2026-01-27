---
hive-console-sdk: patch
hive-router: patch
---

# Fix release issues and conflicts

- Pin dependency `graphql-tools` to avoid release issues and conflicts 
- Re-export `graphql-tools` from `hive-console-sdk` to make it easier to depend directly on the SDK
