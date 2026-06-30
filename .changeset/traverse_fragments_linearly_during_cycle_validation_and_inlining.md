---
hive-router-query-planner: patch
graphql-tools: patch
hive-router-plan-executor: patch
hive-router: patch
node-addon: patch
hive-console-sdk: patch
hive-router-internal: patch
hive-apollo-router-plugin: patch
---

# Fix: Traverse fragments linearly during cycle validation and inlining

GraphQL fragments can spread other fragments, e.g. `fragment A on T { ...B }`. When fragments form a long acyclic chain (A spreads B, B spreads C, and so on for thousands of links), we walked that chain with plain recursion. 

This change prevents the stack from being filled in such cases. 
