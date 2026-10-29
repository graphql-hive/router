---
hive-router: patch
---

# Accept empty `@requiresScopes` and `@policy` groups

Fixed an issue where the router refused to load a supergraph with an empty group in `@requiresScopes` or `@policy`, such as `@requiresScopes(scopes: [[]])` or `@policy(policies: [])`, so every request failed. Composition accepts these values, and the router now treats them as no requirement.

Closes https://github.com/graphql-hive/router/issues/1792
