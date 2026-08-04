---
hive-router-config: minor
hive-router: minor
hive-router-internal: patch
hive-router-plan-executor: patch
---

# Add `jwt.scopes_claim`

Adds a new `jwt.scopes_claim` configuration option that lets you specify which JWT claim the `@requiresScopes` directive should read authorization data from, instead of the hardcoded `scope`/`scopes` claim.

This is useful for identity providers that grant authorization data under a different claim name — for example, Microsoft Entra ID, which issues app roles under a `roles` claim rather than `scope`. Setting `jwt.scopes_claim: roles` allows `@requiresScopes` to authorize requests using Entra app roles without requiring any changes to how the claim is issued.
