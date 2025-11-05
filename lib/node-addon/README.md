# @graphql-hive/router-query-planner

A high-performance Node.js addon containing the query planner of [Hive Router](https://the-guild.dev/graphql/hive/docs/router). This package provides GraphQL Federation query planning capabilities built with Rust and N-API for optimal performance.

## Overview

This node addon contains the query planner of the Hive Router, enabling you to generate [Query Plans used in Apollo Federation](https://www.apollographql.com/docs/graphos/schema-design/federated-schemas/reference/query-plans). It provides a fast, native implementation for planning GraphQL operations across federated schemas.

## Installation

```bash
npm install @graphql-hive/router-query-planner
# or
yarn add @graphql-hive/router-query-planner
# or
pnpm add @graphql-hive/router-query-planner
```

## Platform Support

This package supports the following platforms:

- **Linux**: x86_64 and aarch64
- **macOS**: x64 and arm64 (Apple Silicon)

The package includes prebuilt binaries for all supported platforms. Node.js 20+ and Bun 1+ are supported.

## Usage

### Basic Usage

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

This will generate you a [Query Plan used in Apollo Federation](https://www.apollographql.com/docs/graphos/schema-design/federated-schemas/reference/query-plans).

### Integration with Hive Gateway

Hive Gateway uses this package as an alternative federation query planner in the [`@graphql-hive/router-runtime`](https://github.com/graphql-hive/gateway/blob/main/packages/router-runtime).

To use with Hive Gateway, first install the runtime:

```bash
npm install @graphql-hive/router-runtime
```

Then configure your gateway:

```ts
// gateway.config.ts
import { defineConfig } from "@graphql-hive/gateway";
import { unifiedGraphHandler } from "@graphql-hive/router-runtime";

export const gatewayConfig = defineConfig({
  unifiedGraphHandler,
});
```
