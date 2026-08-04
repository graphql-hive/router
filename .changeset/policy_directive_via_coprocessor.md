---
hive-router-query-planner: minor
hive-router-internal: minor
hive-router-plan-executor: minor
hive-router: minor
---

# Custom authorization rules with the `@policy` directive

Adds support for the federation `@policy` directive, letting a coprocessor decide custom
authorization rules that the router cannot evaluate on its own.

`@policy(policies: [[...]])` takes an OR of AND groups, the same shape as `@requiresScopes`.
Access is granted when every policy of at least one group is granted for the request.

The decision is made in the `graphql.analysis` coprocessor stage, through a single request context
key, `hive::authorization::required_policies` — mirroring Apollo Router's
`apollo::authorization::required_policies` contract. The router seeds it with every policy the
incoming operation depends on, each mapped to `null`. A coprocessor decides by overwriting entries
with `true`/`false`; anything left `null`, or missing from the answer entirely, is denied.

Unauthorized fields are then handled by the existing
`authorization.directives.unauthorized.mode` setting: `filter` (default) nulls them and reports an
`UNAUTHORIZED_FIELD_OR_TYPE` error, `reject` fails the whole operation. As with the other
authorization directives, subgraph requests that would only resolve unauthorized fields are never
sent.

`@policy` is independent of `@authenticated` and `@requiresScopes`: it is enforced even when JWT
authentication is not configured, and when several directives sit on the same field all of them
must be satisfied.

Example coprocessor answer for the `graphql.analysis` stage:

```json
{
  "version": 1,
  "control": "continue",
  "context": {
    "hive::authorization::required_policies": {
      "read_profile": true
    }
  }
}
```
