# @graphql-hive/router-query-planner changelog
## 0.0.9 (2025-12-11)

### Fixes

- chore: Enable publishing of internal crate

## 0.0.8 (2025-12-11)

### Fixes

#### Prevent planner failure when combining conditional directives and interfaces

Fixed a bug where the query planner failed to handle the combination of conditional directives (`@include`/`@skip`) and the automatic `__typename` injection required for abstract types.

## 0.0.7 (2025-12-08)

### Fixes

- Bump dependencies

## 0.0.6 (2025-11-28)

### Fixes

- make supergraph.{path,key,endpoint} optional (#593)

## 0.0.5 (2025-11-28)

### Fixes

- support `@include` and `@skip` in initial fetch node (#591)
- Fixed an issue where `@skip` and `@include` directives were incorrectly removed from the initial Fetch of the Query Plan.

## 0.0.4 (2025-11-24)

### Fixes

#### Avoid extra `query` prefix for anonymous queries

When there is no variable definitions and no operation name, GraphQL queries can be sent without the `query` prefix. For example, instead of sending:

```diff
- query {
+ {
  user(id: "1") {
    name
  }
}
```

## 0.0.3 (2025-11-06)

### Fixes

#### CommonJS bindings

Adding support for CJS.

## 0.0.2 (2025-11-05)

### Features

#### A node addon containing the query planner of hive router

To use in TypeScript, you would go ahead and do something like:

```ts
import {
  QueryPlanner,
  type QueryPlan,
} from "@graphql-hive/router-query-planner";

const supergraphSdl = "<your sdl from file or in code>";

const qp = new QueryPlanner(supergraphSdl);

const plan: QueryPlan = qp.plan(/* GraphQL */ `
  {
    posts {
      title
      author {
        name
      }
    }
  }
`);

// see QueryPlan types in lib/node-addon/src/query-plan.d.ts
```

which will generate you a [Query Plan used in Apollo Federation](https://www.apollographql.com/docs/graphos/schema-design/federated-schemas/reference/query-plans).

Hive Gateway uses it as an alternative federation query planner in the [`@graphql-hive/router-runtime`](https://github.com/graphql-hive/gateway/blob/main/packages/router-runtime).

To use in with Hive Gateway, you first install the runtime

```sh
npm i @graphql-hive/router-runtime
```

```ts
// gateway.config.ts
import { defineConfig } from "@graphql-hive/gateway";
import { unifiedGraphHandler } from "@graphql-hive/router-runtime";

export const gatewayConfig = defineConfig({
  unifiedGraphHandler,
});
```
