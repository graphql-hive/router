# Indexed pipeline: types and terms

The canonical names for the indexed response pipeline, grouped by when each part runs. Paths
are under `bin/router/src/executor/` unless they say otherwise. `plan-log.md` was written
before the rename and keeps the old names; its first table maps them to these.

The **tree-based pipeline** (the `Value` pipeline) reads each subgraph body into a `Value`
tree, merges trees, and projects the client response from the merged tree. The **indexed
pipeline** does the same work without building a tree. When a plan is compiled, every JSON
object gets a fixed layout and every field a numeric index. A request then copies byte ranges
of the subgraph bodies into those places and writes the client response from them. Nested
objects and lists are still there; what's gone is the allocated `Value` tree.

Config: `query_planner.experimental_indexed_pipeline` (env
`QUERY_PLANNER_EXPERIMENTAL_INDEXED_PIPELINE`). Entity reuse:
`traffic_shaping.experimental_entity_reuse` (env `ROUTER_EXPERIMENTAL_ENTITY_REUSE`); it only
applies on the indexed pipeline.

## Core vocabulary

| Term | Meaning |
|---|---|
| **response position** | A place in the response tree known when the plan is compiled (the root, `me`, `me.reviews[]`…). |
| **layout group** | Positions merged with union-find because they need the same object layout (the same object filled by several fetches). The code calls it a *class*. |
| **object layout** (`ObjectLayout`) | The fixed mapping from an object's response keys to field indexes. |
| **field index** | A field's index in its object layout, a `u16`. `NO_FIELD` means the key isn't written at this position. |
| **value cell** (`ValueCell`) | One JSON value, 16 bytes, `Copy`. |
| **object block** | One object's cells: `width` cells starting at `first` in `Arena::cells`. |
| **cell offset** | An absolute index into `Arena::cells` (`first + field_index`). |
| **JSON span** (`JsonSpan`) | A byte range of a retained subgraph body. |
| **entity target** | A concrete object in the response that an entity fetch's result is merged into. |
| **response arena** (`Arena`) | Pooled, per-request storage: bodies, cells, list items and scratch space. |

A field index identifies a field in a layout, a cell offset identifies a storage location, and a
value cell holds the value.

## 1. Compiled once per query plan (`indexed/compile.rs`, `indexed/mod.rs`)

| Name | What it is | Why it exists |
|---|---|---|
| `IndexedResponsePlan` | Everything compiled for one query plan and one projection plan: the object layouts, the root layout, each projection field's `OutputField`, and one `FetchProgram` per fetch. | Turns per-request lookups (keys, types, paths) into indexes worked out once. |
| `IndexedPlanCache` | The `IndexedResponsePlan`s of one cached query plan: the first one, plus up to 16 more for other projection plans (an auth rewrite, another operation that shares the query plan). | Compiles on first use and finds the right plan for a request's projection plan. |
| `Seed` | Data the router has before the plan runs: none, an operation's own data (introspection result, a plugin's start data), or a subscription event. It's compiled as an extra fetch with id `SEED_FETCH`. | Reads start data into cells the same way as a fetch response. |
| `Unsupported` | Why a plan can't be compiled. | Only for logs and tests; such a plan stays on the tree-based pipeline. |
| `Positions` | The union-find over response positions; each class is a layout group. | Every fetch that writes the same object writes into the same layout. |
| `ObjectLayout` | One layout group's fields: the response key of each field index, in order, and the field index of `__typename`. | A field becomes an index instead of a key lookup. |
| `OutputField` | For each projection-plan field: its field index, its `"key":` bytes padded to a 24-byte window, and the `SIMPLE` flag. | The writer copies keys with one fixed-size store and takes a fast loop for plain fields. |
| `FetchProgram` | Per fetch: its `FetchShape`, its entity aliases (`CompiledEntityAlias`), and where a root re-entry fetch merges. | Everything needed to send a fetch and merge its answer. |
| `CompiledEntityAlias` | Per entity alias of a fetch (`_entities`, or `_e0`, `_e1`… in a batch): the field index of its list in `data`, the paths to its entity targets, its `Requires`, its `ReuseProgram`, and its `__typename` rewrites. | Writing representations and merging entities back for one alias. |
| `PathProgram` / `PathStep` | A path in the response resolved to field indexes: `Field(field_index)`, `List`, `TypeCondition`. | Walks the cells to the entity targets without key lookups. |
| `Requires` / `RequiresNode` / `Read` | A `@requires` or key selection resolved to field indexes: the nodes to write a representation from, plus a flat list of everything it reads. | Writing representations, and fingerprinting entity targets without writing them. |
| `TypenameRewrite` | The planner's input rewrite that sends an `@interfaceObject` entity under the interface's name. | Representations name the type the subgraph expects. |
| `Rename` | The planner's output rewrite: a field the fetch aliases, put back under its own key. | Reads it into the right cell. |
| `ReuseProgram` / `ReuseSource` | The planner's `EntityReuse` resolved to field indexes: where entities of this type may already be, and which fields to copy. | Entity reuse: copy instead of fetching again. |
| `FetchShape` / `ShapeObject` / `ShapeNode` | One fetch's query, as the parser reads it. Objects list their nodes (fields) in query order; each node has its key bytes, field index, child object and flags (`opaque_json` for a custom scalar). It also keeps schema field names and type conditions for plugin walks. | Reads a body in one pass, matching keys by prefix compare instead of string compare. |
| `Verify` / `VerifyObject` | A *passthrough* check: what a subtree must look like (keys in order, non-null where required, enum values) for the client to get the subgraph's bytes unchanged. | Keeps a whole subtree as one JSON span when it checks out. |
| `Collected` | A response key with its field name and the selection sets under it (shared with the tree-based pipeline's fetch walk). | Collects a selection set's fields once, for both pipelines. |
| `NO_FIELD`, `NO_OBJECT`, `SEED_FETCH`, `KEY_WINDOW` | Sentinels and sizes. | |

## 2. Per-request storage (`indexed/store.rs`)

| Name | What it is | Why it exists |
|---|---|---|
| `ValueCell` | `Missing`, `Null`, `True`, `False`, `RawJson(JsonSpan)`, `Object { layout, first }`, `List { start, len }`. Checked to be 16 bytes at compile time. | The unit of storage. Nothing to drop, so clearing a request is O(1). |
| `JsonSpan` | `{ body, start, len }`: a number, a quoted string, a custom scalar, or a passthrough subtree, as the subgraph wrote it. | Values are never decoded, only copied. |
| `Arena` | One request's storage: `bodies`, object blocks (`cells`), list items (`items`), and scratch space for walks (`found`), dedupe, fingerprints and plugin edits. Kept in a per-thread pool (4 arenas; one that grew past 1M cells isn't kept). | Reused across requests, so a request allocates almost nothing. |
| `EDITS_BODY` / `Arena::edits` | The body index of values a plugin set, and the buffer that holds them. | A plugin edit is just another JSON span. |
| `CellView` | A cell plus its arena and layouts, read like a value. | Demand control counts actual cost on cells. |

## 3. Running the plan (`execution/plan/indexed.rs`)

| Name | What it is | Why it exists |
|---|---|---|
| `usable` / `execute` | Picks the indexed pipeline for a request and runs it: plugins' start hooks, the plan, demand control, end hooks, writing. | The entry point, next to the tree-based pipeline's. |
| `Run` | A request in flight: its `IndexedResponsePlan`, arena, root cell, whether passthrough is on, and the shared plan for plugin walks. | The state the plan walk changes. |
| *wave* | The fetches of one `Parallel` node (or one fetch), sent together and merged as they arrive. | |
| `PendingMerge` | The rest of a job once its request is sent: what its response is merged into, `Root` (the root, or a re-entry path) or `Entities` (one or more aliases), and how to read it. Parked while the request is in flight, taken once when it finishes, consumed by the merge. | Keeps what's needed to read and merge a response until it arrives. |
| `EntityTargetMap` | Every entity target of an alias, in walk order, with the index of the entity it gets (`NO_ENTITY` for none), as `entries`, and where each path's targets end. | Maps the `_entities` list back to the entity targets. |
| `AliasMerge` | One alias of a job: its program, name, paths and `EntityTargetMap`. | |
| `ContextGroupPartition` | An aliased `@fromContext` request (several context groups in one request): how many entities each group sent. | Reads the aliased answer back as one list per key. |
| `FetchProcessingContext` | A job's subgraph name, path, hash and kind, kept apart from its response. | Lets a hook's replacement response go through the same processing. |
| *representation* | The JSON object sent in `representations` for one entity. | |
| `RepresentationWriter` | Writes a representation from cells (a port of `project_requires`). | |
| `AppliedSelections` | Which selections applied while writing one representation. | Decides the `__typename`-only and missing-field cases. |
| `Fingerprint` / `Mix` | A hash of everything a representation would read, and the small hasher it uses. | Dedupes entity targets without writing their representations. |
| *entity reuse* | Copying an entity's fields from an object already in the response instead of fetching it. | Fewer subgraph calls; experimental. |

## 4. Reading bodies (`indexed/parse.rs`)

| Name | What it is | Why it exists |
|---|---|---|
| `parse_body` / `ParsedBody` | Reads a subgraph response into cells with a fetch's `FetchShape`: `data`, `errors` (via sonic), and the byte range of `extensions`. | One pass, validated like JSON, nothing decoded. |
| `parse_data` | Reads a bare `data` object (introspection result, plugin start data). | |
| `parse_aliased_body` / `AliasedBody` | Reads an aliased context request's answer. | |
| `Parser` | The reader: SIMD string scanning, key prefix matching, a lookahead of 4 fields, a depth limit of 250. | |
| *passthrough* | A subtree that passes its `Verify` check is kept as one `RawJson` cell. Off when demand control or end hooks need to see every value. | Copies a whole subtree at once. |

## 5. Writing the client response (`indexed/write.rs`)

| Name | What it is | Why it exists |
|---|---|---|
| `write_response` / `Writer` | Writes `{"data":…,"errors":…,"extensions":…}` from cells, walking the projection plan field by field: conditions, type guards, enum checks, `__typename`, forced nulls, null bubbling. | Same output as the tree-based pipeline's projection, byte for byte. |
| `LazyTypename` | A type name that is known, or found the first time it's needed. | Skips schema lookups that aren't needed. |
| `Failure` | Why a field isn't written as is (skipped, wrong type, invalid enum, fatal). | |
| `Null` | Whether a written `null` bubbles up. | |

## 6. Plugins (`plugins/response_data.rs`, `indexed/walk.rs`, `projection/walk.rs`)

| Name | What it is | Why it exists |
|---|---|---|
| `ResponseData` | The response data in `on_execute`'s end hook, on either pipeline (cells or a `Value`). | One plugin API for both pipelines. |
| `SubgraphData` | One fetch's data in `on_subgraph_execute`'s end hook, before it's merged. | |
| `DataValue` | One value during a walk: what it is (field, type, path, nullability) and the edits allowed (`set`, `set_null`, `set_null_with_error`). | Edits that keep the response valid for the query and schema. |
| `DataEditError` | Why an edit was refused. | |
| `Place` | Where a value lives: `Cell` (an object block or list item) or a `&mut Value`. | |
| `Nullability` | From the projection plan (client data) or looked up in the schema (fetch data). | Decides whether a null bubbles. |
| `PathFrame` / `response_data::Step` | The walk's path, one frame per level on the stack. | Builds the path only when an error needs it. |
| `Walker` / `FetchWalker` | The walks over cells (client data and fetch data). | |
| `StartData` (`execution/plan.rs`) | What the plan starts from: JSON bytes, or a subscription event. | |
| `SubgraphExtensions`, `run_end_hooks` (`hooks/on_subgraph_execute.rs`) | A subgraph's extensions as the hook reads them, and running the hooks once the body is read. | Hooks run after the body is read, so it's read only once. |
| `SubgraphResponse::unparsed_body` / `has_unparsed_body()`, `SubgraphExecutionRequest::unparsed_body` | A response whose body nothing has read yet, and the request flag that asks an executor for one. | The indexed pipeline reads bodies itself. |

## 7. Related, outside the indexed pipeline

| Name | Feature | What it is |
|---|---|---|
| `ResponseShape` (`response/shaped.rs`) | Shape-guided JSON parser | The tree-based pipeline's parser: reads a body into a `Value` tree knowing the fetch's query. Used on the default pipeline. Its custom-scalar flag is also `opaque_json`. |
| `EntityReuse` (`query_planner/planner/plan_nodes`) | In-request entity reuse | The planner's side: source paths and the fields to copy. Planned only with entity reuse and the indexed pipeline both on. |
| `H1Pool` / `Lease` (`executors/h1_pool.rs`) | Strict HTTP/1 pool | Opens a connection only when every connection of its pool is busy; shared or per worker, following `http_client_mode`. Not part of the indexed pipeline. |

## Words still used for more than one thing

- **shape**: `FetchShape` (indexed parser), `ResponseShape` (shape-guided parser), `ShapeCursor` (nullability in the projection plan), and `Verify::shape`.
- **alias**: an entity alias of a fetch (`_e0`), a field alias in a query, and aliased context requests.
- **program**: `FetchProgram`, `PathProgram`, `ReuseProgram`. These are compiled data, not code; kept until their responsibilities are looked at more closely.
- **typename**: `LazyTypename` here, while the tree-based pipeline's projection keeps its own `TypeName`.

## Test baseline with the indexed pipeline forced on

```bash
QUERY_PLANNER_EXPERIMENTAL_INDEXED_PIPELINE=true ROUTER_EXPERIMENTAL_ENTITY_REUSE=true cargo test_e2e
```

Expected: 844/848. The four failures below are expected; anything else is a regression.

| Test | Why it fails here |
|---|---|
| `persisted_documents::storage_file::file_watch_works` | Fails locally with any config. |
| `entity_batching::…::measure_bench_operation_subgraph_calls` | Entity reuse makes fewer subgraph calls than the snapshot counts (passes with reuse off). |
| `entity_batching::…::max_aliases_per_request_splits_batches` | Same: fewer calls with reuse on. |
| `telemetry::logging::json_log_default_attrs` | The "subgraph execution completed" log leaves out `error_count` and `partial_response` for a body the pipeline hasn't read yet. |

With the default config, `cargo test_e2e` is 847/848 (only `file_watch_works`).
