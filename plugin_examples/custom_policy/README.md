# Custom `@policy` authorization

Demonstrates deciding [`@policy`](https://the-guild.dev/graphql/hive/docs/router/security/policy)
policies from a plugin instead of a coprocessor. Both go through the exact same mechanism: before
the `graphql.analysis` stage runs, the router walks the operation and publishes every policy it
depends on to the request context, as `hive::authorization::required_policies` - a map of
`policy -> null`. Whoever answers the `graphql.analysis` stage - a
[coprocessor](https://the-guild.dev/graphql/hive/docs/router/customizations/coprocessors/stages-and-protocol)
over HTTP, or an in-process [plugin](https://the-guild.dev/graphql/hive/docs/router/customizations/plugin-system)
like this one - decides which of them are granted by overwriting entries with `true`/`false`.
Anything left `null`, or missing from the answer entirely, is denied and the field it protects is
filtered out (or the whole operation rejected, depending on `authorization.directives.unauthorized.mode`).

The example schema gates `Product.inStock` behind a `read_inventory` policy:

```graphql
inStock: Boolean @policy(policies: [["read_inventory"]])
```

The plugin grants every policy an operation depends on when the request carries an
`x-user-role: admin` header, and denies all of them otherwise - see
[`src/plugin.rs`](./src/plugin.rs).

## How to run?

```bash
cargo run --package custom-policy-plugin-example
```

Then, without the header, `inStock` comes back `null` with an `UNAUTHORIZED_FIELD_OR_TYPE` error:

```bash
curl http://localhost:4000/graphql \
  -H 'content-type: application/json' \
  -d '{"query": "{ topProducts(first: 1) { upc inStock } }"}'
```

With the header, it resolves normally:

```bash
curl http://localhost:4000/graphql \
  -H 'content-type: application/json' \
  -H 'x-user-role: admin' \
  -d '{"query": "{ topProducts(first: 1) { upc inStock } }"}'
```

## The plugin

See the [`on_graphql_analysis` hook reference](https://the-guild.dev/graphql/hive/docs/router/customizations/plugin-system/hooks#on_graphql_analysis)
for the full payload API.

```rust
async fn on_graphql_analysis<'exec>(
    &'exec self,
    payload: &mut OnGraphqlAnalysisHookPayload<'exec>,
) -> OnGraphqlAnalysisHookResult {
    let required_policies: Vec<String> = match payload.request_context.read() {
        Ok(read) => read
            .authorization()
            .required_policies()
            .map(|policies| policies.keys().cloned().collect())
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    };

    if required_policies.is_empty() {
        return OnGraphqlAnalysisHookResult::Proceed;
    }

    let is_admin = payload
        .router_http_request
        .headers
        .get(ROLE_HEADER)
        .and_then(|value| value.to_str().ok())
        == Some(ADMIN_ROLE);

    if let Ok(mut write) = payload.request_context.write() {
        let mut authorization = write.authorization();
        for policy in required_policies {
            authorization.set_policy_decision(policy, is_admin);
        }
    }

    OnGraphqlAnalysisHookResult::Proceed
}
```

A real plugin would decide each policy on its own merits (a permissions service, a database lookup,
claims already present on the request context from an earlier hook) instead of granting every
required policy to any admin - the router doesn't care how the decision is made, only that
`set_policy_decision` gets called for the policies you want to grant before this hook returns.

## Why a plugin instead of a coprocessor?

Both are valid. A [coprocessor](https://the-guild.dev/graphql/hive/docs/router/customizations/coprocessors/stages-and-protocol)
is a separate service, reachable over HTTP, useful when the authorization logic is shared across
services or owned by a different team. A [plugin](https://the-guild.dev/graphql/hive/docs/router/customizations/plugin-system)
runs in-process, with no network hop, and is a better fit when the router binary itself is already
the right place to own the decision.

## Further reading

- [`@policy` directive](https://the-guild.dev/graphql/hive/docs/router/security/policy)
- [Plugin system](https://the-guild.dev/graphql/hive/docs/router/customizations/plugin-system)
- [`on_graphql_analysis` hook reference](https://the-guild.dev/graphql/hive/docs/router/customizations/plugin-system/hooks#on_graphql_analysis)
