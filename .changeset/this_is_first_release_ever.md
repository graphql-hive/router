---
node-addon: minor
---

# A node addon containing the query planner of hive router

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
