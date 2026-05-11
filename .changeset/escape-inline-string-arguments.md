---
hive-router-query-planner: patch
hive-router: patch
node-addon: patch
---

# Escape inline string arguments when emitting subgraph operations

Fixes a bug where string values inlined as arguments in subgraph operations were not re-escaped per the GraphQL spec. When an incoming operation contained a string literal whose decoded value carried a quote or backslash (for example `value: "\"aValue\""`), the router forwarded the argument to the subgraph as `value: ""aValue""`, producing invalid GraphQL. The same went for newlines, tabs, and other control characters.

Now the characters are escaped properly per GraphQL spec [here](https://spec.graphql.org/draft/#StringCharacter).
