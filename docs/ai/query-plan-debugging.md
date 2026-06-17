# Hive Router Query Planning Debugging

Query planning turns a client operation into a plan for fetching data from subgraphs. Each planning stage has a `cargo dev` command to inspect it.

The main artifacts are:

1. **Consumer schema**
2. **Internal dependency graph**
3. **Best fetch paths**
4. **Fetch graph**
5. **Query plan**

Normalized query and response projection plan are in [execution-debugging.md](execution-debugging.md).

---

## Flow

```txt
normalized operation + supergraph schema
        ↓
build internal dependency graph
        ↓
find best fetch paths per leaf field
        ↓
build query tree (merge best paths)
        ↓
build fetch graph (subgraph fetch steps)
        ↓
build final query plan
```

Each stage feeds into the next. You can inspect any stage on its own.

---

## 1. Consumer schema

The schema that clients see. Federation directives like `@key`, `@requires`, `@provides` are removed.

```sh
cargo dev consumer_schema <supergraph.graphql>
```

Example:

```sh
cargo dev consumer_schema supergraph.graphql
```

---

## 2. Internal dependency graph

The planner builds a graph showing how types and fields connect across subgraphs (keys, requires, provides). Output is in Graphviz (DOT) format.

```sh
cargo dev graph <supergraph.graphql>
```

Example:

```sh
cargo dev graph supergraph.graphql > graph.dot
dot -Tsvg graph.dot -o graph.svg
```

If this graph looks wrong, the bug is in how the supergraph schema is parsed or how the graph is built.

---

## 3. Best fetch paths

For each leaf field in the operation, the planner finds all valid paths through subgraphs. A path says which subgraph can provide a field and how to reach it.

```sh
cargo dev paths <supergraph.graphql> <operation.graphql>
```

Example:

```sh
cargo dev paths supergraph.graphql operation.graphql
```

If wrong paths are picked (or good paths are missed), the bug is in the walker or path-finding code.

---

## 4. Fetch graph

The fetch graph sits between raw paths and the final plan. It shows subgraph fetch stages — what each subgraph is asked, in what order, and how results move between stages.

```sh
cargo dev fetch_graph <supergraph.graphql> <operation.graphql>
```

Example:

```sh
cargo dev fetch_graph supergraph.graphql operation.graphql
```

If the fetch graph looks wrong (bad subgraph assignments, missing dependencies), the bug is in `build_fetch_graph_from_query_tree`.

---

## 5. Query plan

The final plan — the list of subgraph requests with their order and selections.

```sh
cargo dev plan <supergraph.graphql> <operation.graphql>
```

Use `--json` for JSON output.

Example:

```sh
cargo dev plan supergraph.graphql operation.graphql
cargo dev plan supergraph.graphql operation.graphql --json
```

If the fetch graph is right but the plan is wrong, the bug is in `build_query_plan_from_fetch_graph`.

---

## Debugging workflow

Check stages in order from earliest to latest:

```txt
1. Check normalized query     (cargo dev normalize)
2. Check consumer schema       (cargo dev consumer_schema)
3. Check internal graph        (cargo dev graph)
4. Check best fetch paths      (cargo dev paths)
5. Check fetch graph           (cargo dev fetch_graph)
6. Check final query plan      (cargo dev plan)
7. Check response projection   (cargo dev projection)
```

Don't skip ahead. Find the first stage that goes wrong.

---

## Related commands

| Command | What it does |
|---|---|
| `cargo test_qp` | Run all query planner tests |
| `cargo test_qpe` | Run all plan executor tests |
| `cargo run -p graphql-differential -- <base> <candidate> <schema>` | Compare two GraphQL endpoints |
