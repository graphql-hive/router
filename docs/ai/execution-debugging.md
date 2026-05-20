# Hive Router GraphQL Execution Debugging

Hive Router execution has three major planning/debugging artifacts:

1. **Normalized query**
2. **Query plan**
3. **Response projection plan**

Understanding the difference between them is critical.

---

## Mental model

When Hive Router receives a GraphQL operation, it does not execute the raw query directly.

Instead, the flow is roughly:

```txt
client GraphQL operation
        ↓
normalize operation
        ↓
create query plan
        ↓
create response projection plan
        ↓
execute subgraph requests
        ↓
collect and merge subgraph responses
        ↓
project merged data into final client response
```

---

## 1. Normalized query

The normalized query is the canonical form of the client operation.

It resolves and simplifies things like:

* fragments
* inline fragments
* aliases
* field merging
* directive effects such as `@skip` and `@include`
* type conditions
* selection set structure

The normalized query is the best place to check whether Hive Router understood the client operation correctly before federation planning starts.

To inspect it:

```sh
cargo dev normalize <supergraph.graphql> <operation.graphql>
```

Example:

```sh
cargo dev normalize supergraph.graphql operation.graphql
```

If the normalized query is wrong, the bug is likely before query planning.

---

## 2. Query plan

The query plan describes how Hive Router will fetch data from subgraphs.

Its job is to answer:

> Which subgraph requests need to be executed, in what order, and with what selections?

To inspect it:

```sh
cargo dev plan <supergraph.graphql> <operation.graphql>
```

Example:

```sh
cargo dev plan supergraph.graphql operation.graphql
```

If the normalized query is correct but the query plan is wrong, the bug is likely in federation query planning.

---

## 3. Response projection plan

The response projection plan describes how Hive Router turns collected and merged subgraph data into the final GraphQL response sent back to the client.
The query plan fetches data.
The response projection plan shapes data.

Its job is to answer:

> Given the merged internal data, how do we produce the exact response shape requested by the client?

The projection plan is responsible for:

* applying aliases
* preserving client response shape
* selecting only requested fields
* handling nested objects and lists
* handling inline fragments and type conditions
* handling `__typename`
* mapping merged subgraph data back to the normalized operation
* removing internal fields that were only needed for planning

To inspect it:

```sh
cargo dev projection <supergraph.graphql> <operation.graphql>
```

Example:

```sh
cargo dev projection supergraph.graphql operation.graphql
```

If the normalized query and query plan are correct, but the final response is wrong, the bug is likely in response projection.

---

## Debugging workflow

Always debug in this order:

```txt
1. Check normalized query
2. Check query plan
3. Check response projection plan
4. Check actual subgraph responses
5. Check merge behavior
6. Check final projected response
```

Do not jump straight into execution code.

Most bugs become much easier to locate once you know which artifact first becomes incorrect.
