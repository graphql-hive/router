---
hive-router: major
---

# Custom authorization rules with the `@policy` directive

Adds support for the federation `@policy` directive, letting a coprocessor decide custom
authorization rules the router can't evaluate on its own.

`@policy(policies: [[...]])` takes an OR of AND groups, the same shape as `@requiresScopes`, and is
enforced independently of `@authenticated`/`@requiresScopes` - it stays active even without JWT
configured, and when several directives sit on a field all of them must be satisfied.

The router publishes every policy the operation depends on to
`hive::authorization::required_policies` before the `graphql.analysis` coprocessor stage; the coprocessor decides by
overwriting entries with `true`/`false`. Anything left `null`, or missing from the answer, is
denied and handled like any other unauthorized field, via
`authorization.directives.unauthorized.mode`.


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
