# Fused Response Deserialization And Projection

## Goal

Replace the current response pipeline:

```text
subgraph bytes
  -> deserialize into recursive Value tree
  -> merge Value trees
  -> project operation selection from Value
  -> serialize projected JSON response
  -> drop Value tree
```

with a Grafbase-style pipeline:

```text
subgraph bytes
  -> deserialize through compiled response shapes
  -> write into a normalized flat response store
  -> serialize final client response from the flat store
```

The new design should be the primary execution model, not an optional CT-only fast path. The existing `Value` tree path can remain temporarily as a migration fallback while the new path reaches feature parity, but the target architecture is a full replacement for response materialization, merging, projection, and serialization.

## Motivation

Profiles show that local shaped-deserialization optimizations are hitting an architectural ceiling. Even after avoiding object sorting, adding shaped deserialization, cursor lookup, simple projection, and typed shape experiments, the full path still spends substantial time in:

- building intermediate `Value::Object` trees
- projecting from those trees
- serializing projected output
- dropping the recursive tree

Grafbase avoids this class of work by deserializing into response storage guided by prepared shapes. Hive Router should move in the same direction: deserialize once into a normalized response representation that is already aligned with the final client response shape.

## Current State

Today the executor roughly does:

1. Each subgraph HTTP response is deserialized into `SubgraphResponse { data: Value, errors, extensions, ... }`.
2. Fetch results are merged through `response::merge::deep_merge` over recursive `Value` trees.
3. Final response projection walks `FieldProjectionPlan` against the merged `Value` tree.
4. Projection writes JSON bytes into a response buffer.
5. The recursive `Value` tree is dropped.

This means large responses pay for at least two structural walks after deserialization: projection and drop. Merged multi-fetch responses also pay for recursive object/list merging.

## Target Architecture

Introduce a normalized flat response store and make subgraph deserialization write into it directly.

```text
QueryPlan + FieldProjectionPlan + SchemaMetadata
  -> CompiledResponseWritePlan

Subgraph bytes + CompiledResponseWritePlan
  -> FlatResponseStore updates

FlatResponseStore
  -> final client JSON response
```

The central idea is that each fetched subgraph response is decoded through a compiled write plan that knows:

- where each subgraph field should land in response storage
- how list and object nesting maps into storage
- final response key order
- nullability and null propagation rules
- fragment/type-condition behavior
- custom scalar raw JSON boundaries
- merge targets for `Flatten`, entity fetches, and batch fetches

## Core Data Model

Add a flat response store that replaces recursive `Value` as the execution response data model.

```rust
pub struct FlatResponseStore<'a> {
    values: Vec<FlatValue<'a>>,
    object_fields: Vec<FlatObjectField>,
    list_items: Vec<FlatValueId>,
    root: FlatValueId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FlatValueId(u32);

pub enum FlatValue<'a> {
    Null,
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    String(Cow<'a, str>),
    RawJson(Cow<'a, str>),
    Object { fields: Range<u32> },
    List { items: Range<u32> },
    Missing,
    Inaccessible,
}

pub struct FlatObjectField {
    pub response_key: ResponseKeyId,
    pub value: FlatValueId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResponseKeyId(u32);
```

Keep response key strings in a compact table:

```rust
pub struct ResponseKeys {
    keys: Vec<Box<str>>,
    serialized_keys: Vec<Box<[u8]>>, // precomputed: "field":
}
```

The store should support borrowed strings/raw JSON by retaining subgraph response bytes for the lifetime of the final response, similar to the current `SubgraphResponse.bytes` self-borrowing model.

## Compiled Plans

Create a compiled response write plan from the query plan, subgraph fetch operations, final projection plan, and schema metadata.

```rust
pub struct ResponseWritePlan {
    pub root: RootWritePlan,
    pub fetches: ResponseWritePlanRegistry,
    pub response_keys: ResponseKeys,
    pub estimated_response_size: usize,
}

pub struct ResponseWritePlanRegistry {
    pub plans_by_fetch_id: Vec<(i64, FetchWritePlan)>,
}

pub struct FetchWritePlan {
    pub fetch_id: i64,
    pub service_name: Box<str>,
    pub target: FetchTarget,
    pub data: ValueWritePlan,
    pub custom_scalar_paths: Option<CustomScalarPaths>,
}

pub enum FetchTarget {
    Root,
    Flatten { path: Vec<ResponsePathSegment> },
    EntityBatch { aliases: Vec<EntityAliasWritePlan> },
}
```

The value write plan should describe both incoming subgraph response shape and final storage layout.

```rust
pub enum ValueWritePlan {
    Leaf(LeafWritePlan),
    Object(ObjectWritePlan),
    List(ListWritePlan),
    PolymorphicObject(PolymorphicObjectWritePlan),
    Skip,
}

pub struct LeafWritePlan {
    pub response_key: ResponseKeyId,
    pub source_key: Box<str>,
    pub nullability: FieldNullability,
    pub scalar_kind: ScalarKind,
    pub custom_scalar: bool,
}

pub struct ObjectWritePlan {
    pub response_key: ResponseKeyId,
    pub source_key: Box<str>,
    pub nullability: FieldNullability,
    pub fields: Vec<FieldWritePlan>,
    pub object_type_name: Box<str>,
}

pub struct FieldWritePlan {
    pub source_key: Box<str>,
    pub response_key: ResponseKeyId,
    pub value: ValueWritePlan,
    pub nullability: FieldNullability,
    pub condition: Option<CompiledWriteCondition>,
}

pub struct ListWritePlan {
    pub response_key: ResponseKeyId,
    pub source_key: Box<str>,
    pub nullability: FieldNullability,
    pub item: Box<ValueWritePlan>,
}
```

## Plan Compilation

The response write plan compiler should replace the current split between `SubgraphResponseShapeRegistry` and `FieldProjectionPlan` at execution time.

Inputs:

- `QueryPlan`
- fetch operation ASTs
- `FieldProjectionPlan`
- `SchemaMetadata`
- subgraph schema metadata/state
- custom scalar paths

Output:

- one global `ResponseWritePlan`
- one `FetchWritePlan` per fetch node
- stable `ResponseKeyId`s and pre-escaped response keys
- type/nullability metadata needed for write-time null propagation

Compilation responsibilities:

- Preserve final client field order.
- Map subgraph response keys to final response keys.
- Compile aliases once.
- Compile inline fragments and fragment spreads into conditions or polymorphic object plans.
- Compile `__typename` handling.
- Compile custom scalar terminal paths.
- Compile flatten and entity-batch targets.
- Compile missing-field/null-bubbling behavior.
- Precompute lookup helpers for object fields.

## Deserialization Into Store

Each subgraph response is decoded with a `FetchWritePlan`.

```rust
pub fn deserialize_fetch_into_store(
    bytes: Bytes,
    plan: &FetchWritePlan,
    store: &mut FlatResponseStore<'static>,
) -> Result<FetchDecodeResult, SubgraphExecutorError>
```

`FetchDecodeResult` should include:

- decoded errors
- decoded extensions
- retained bytes
- status/headers metadata
- whether null propagated to the fetch target

GraphQL response envelope handling:

- `data`: write into store according to `FetchWritePlan.data`
- `errors`: deserialize normally into `Vec<GraphQLError>`
- `extensions`: deserialize into either flat value or existing `Value` initially
- unknown fields: skip with `IgnoredAny`

Object handling:

- Use a visitor over `MapAccess`.
- Match incoming keys to compiled fields.
- Write known fields into temporary object slots.
- Skip unknown fields.
- After the object map ends, materialize object fields in final client response order.
- Fill missing nullable fields with `Null`.
- Propagate missing non-null fields.

List handling:

- Use a visitor over `SeqAccess`.
- Write each item into `list_items`.
- Apply item nullability propagation.
- Materialize a list value as a range of item ids.

Leaf handling:

- Convert scalar callbacks directly into `FlatValue` variants.
- For terminal custom scalars, capture raw JSON into `FlatValue::RawJson`.
- For strings, borrow from retained bytes when possible.
- For numbers, avoid string round-trips.

## Merge And Flatten Semantics

The flat store must replace recursive `deep_merge`.

Add store-level merge operations:

```rust
impl<'a> FlatResponseStore<'a> {
    pub fn merge_value(&mut self, target: FlatValueId, source: FlatValueId);
    pub fn merge_object(&mut self, target: FlatValueId, source: FlatValueId);
    pub fn merge_list_by_index(&mut self, target: FlatValueId, source: FlatValueId);
    pub fn find_path_targets(&self, path: &[ResponsePathSegment]) -> Vec<FlatValueId>;
}
```

Merge rules should match current behavior:

- `source == Null`: no-op
- object + object: merge fields by response key
- list + list: merge pairwise by index
- otherwise: replace target with source

Flatten execution:

- Resolve flatten path against the flat store.
- For each target object/list item, decode fetch response and merge into the target.
- Entity fetches should map returned entities back to target object refs by representation order/hash, as today.

## Projection Replacement

The final serializer should no longer re-project from `Value` for the new pipeline.

Instead, serialization walks the flat store in final response shape order:

```rust
pub fn serialize_store_response(
    store: &FlatResponseStore,
    errors: &[GraphQLError],
    extensions: &ExecutionResultExtensions<'_>,
    buffer: &mut Vec<u8>,
) -> Result<(), ProjectionError>
```

Serialization responsibilities:

- Write `{ "data": ... }`.
- Serialize object fields in stored order, which should already be final client order.
- Serialize lists by item range.
- Serialize scalars directly.
- Copy `RawJson` directly.
- Append `errors` and `extensions` exactly as current `project_by_operation` does.

Current `project_by_operation` remains during migration and for tests, but new execution should produce final output through `serialize_store_response`.

## Null Propagation

Use explicit propagation decisions during decode and merge.

```rust
pub enum WriteDecision {
    Keep(FlatValueId),
    PropagateNull,
}
```

Rules:

- Nullable null writes `FlatValue::Null`.
- Non-null null returns `PropagateNull`.
- Missing nullable field writes `FlatValue::Null`.
- Missing non-null field returns `PropagateNull`.
- Non-null list item propagation propagates to the list.
- Non-null object field propagation propagates to the object.
- Root propagation writes `data: null`.

The behavior must match current projection null bubbling tests.

## Type Conditions And Polymorphism

Polymorphic positions must support interfaces, unions, and fragments.

Compile polymorphic object write plans:

```rust
pub struct PolymorphicObjectWritePlan {
    pub response_key: ResponseKeyId,
    pub source_key: Box<str>,
    pub nullability: FieldNullability,
    pub typename_key: Option<Box<str>>,
    pub variants: Vec<ObjectVariantWritePlan>,
    pub fallback: Option<ObjectWritePlan>,
}

pub struct ObjectVariantWritePlan {
    pub type_condition: TypeCondition,
    pub object: ObjectWritePlan,
}
```

Runtime behavior:

- Prefer `__typename` when present.
- If type is statically known, avoid lookup.
- If type is missing and required for conditions, fallback to current safe behavior or mark invalid.
- Only write fields whose conditions match.

This should replace projection-time type-condition checks for the new pipeline.

## Errors And Extensions

Maintain current behavior:

- Collect subgraph GraphQL errors from each response.
- Preserve error extensions.
- Apply any existing path/location behavior unchanged.
- Append final errors to the serialized response after `data`.
- Preserve extensions behavior.

Initially, `extensions` can remain as existing `Value` if that reduces scope. Later it can move into `FlatResponseStore`.

## Custom Scalars And Raw JSON

Terminal custom scalar paths must preserve raw JSON exactly.

Use `sonic_rs::LazyValue` or serde raw-value support where available:

```rust
let raw = LazyValue::deserialize(deserializer)?;
store.push_raw_json(raw.as_raw_cow());
```

Non-terminal custom scalar paths should keep structured decoding below the path, matching current `CustomScalarPaths` behavior.

## Execution Integration

### Query Plan Payload

Extend cached query plan payload:

```rust
pub struct QueryPlanPayload {
    pub query_plan: Arc<QueryPlan>,
    pub subgraph_response_shapes: Arc<SubgraphResponseShapeRegistry>, // temporary
    pub response_write_plan: Arc<ResponseWritePlan>,
}
```

Eventually remove `subgraph_response_shapes` once the flat pipeline fully replaces shaped `Value` deserialization.

### Subgraph Executor API

The current executor returns `SubgraphResponse`. For the flat pipeline, response bytes must be decoded into a shared execution store.

Preferred long-term API:

```rust
pub enum SubgraphExecutionResult<'a> {
    Decoded {
        errors: Option<Vec<GraphQLError>>,
        extensions: Option<FlatValueId>,
        headers: Option<Arc<HeaderMap>>,
        status: Option<StatusCode>,
        retained_bytes: Option<Bytes>,
    },
}
```

Execution owns the `FlatResponseStore` and passes mutable decode targets to fetch jobs.

If async boundaries make mutable store ownership hard, decode each fetch into a `FlatResponsePart` and merge parts into the main store after the fetch completes.

```rust
pub struct FlatResponsePart<'a> {
    pub store: FlatResponseStore<'a>,
    pub target: FetchTarget,
    pub errors: Option<Vec<GraphQLError>>,
    pub extensions: Option<FlatValueId>,
    pub bytes: Option<Bytes>,
    pub headers: Option<Arc<HeaderMap>>,
    pub status: Option<StatusCode>,
}
```

This is likely easier to integrate with parallel fetches.

### Plan Execution

Replace execution result accumulation:

- Current: fetch jobs return `SubgraphResponse { data: Value }`, then merge `Value`.
- New: fetch jobs return `FlatResponsePart`, then merge part store into main `FlatResponseStore` by `FetchTarget`.

Final execution output uses `serialize_store_response`.

## Migration Strategy

This is a full-solution design, but implementation should still be staged for correctness.

Stage 1: Introduce flat store and serializer.

- Add `FlatResponseStore`.
- Add serialization tests independent of subgraph execution.
- Verify final JSON output order and scalar serialization.

Stage 2: Compile response write plans.

- Build `ResponseWritePlan` from existing query/projection plans.
- Keep shaped `Value` path working in parallel during development.
- Add snapshot/debug formatting for plans.

Stage 3: Decode root fetches into `FlatResponsePart`.

- Implement GraphQL envelope deserializer.
- Implement object/list/leaf write visitors.
- Add correctness tests comparing with current `Value` path.

Stage 4: Implement flat merge and flatten.

- Replace `deep_merge` semantics with flat-store merge.
- Support `Flatten` paths.
- Support parallel/sequence execution merging.

Stage 5: Implement entity and batch fetch support.

- Preserve representation-to-result mapping.
- Support `_entities` and aliased batch entities.
- Preserve current entity null/missing behavior.

Stage 6: Switch executor to flat pipeline by default.

- Use old `Value` path only as temporary fallback for unsupported cases.
- Track fallback reasons with debug logs/metrics.

Stage 7: Remove old path after parity.

- Remove `SubgraphResponseShapeRegistry` if unused.
- Remove shaped `Value` deserialization if no longer needed.
- Keep `Value` only where still needed for extensions/plugins/backward-compatible APIs.

## Correctness Test Matrix

Every case should compare flat pipeline output with current pipeline output.

Required tests:

- scalar leaf fields
- nested objects
- lists of scalars
- lists of objects
- nested lists
- aliases
- field order preservation
- missing nullable field
- missing non-null field
- nullable object value is null
- non-null object value is null
- nullable list item is null
- non-null list item is null
- root null propagation
- extra unknown subgraph fields skipped
- custom scalar terminal raw JSON
- custom scalar nested structured paths
- `__typename`
- inline fragments
- named fragment spreads
- interface fragment conditions
- union fragment conditions
- enum validation behavior
- `@include` and `@skip`
- subgraph GraphQL errors
- response extensions
- single fetch
- sequence fetches
- parallel fetches
- flatten fetches
- entity fetches
- batch entity fetches
- output rewrites
- introspection fallback or support
- subscriptions fallback or support

## Benchmark Plan

Add benchmarks:

```text
ct_response/flat_deserialize_root_fetch
ct_response/flat_full_execute_serialize
ct_response/flat_store_serialize_only
ct_response/flat_merge
```

Compare against existing:

```text
ct_response/deserialize_and_drop
ct_response/deserialize_shaped_and_drop
ct_response/project_only
ct_response/full_deserialize_project_drop
ct_response/full_shaped_deserialize_project_drop
ct_response/drop_value_tree
ct_response/drop_shaped_value_tree
```

Success criteria:

- `flat_full_execute_serialize` beats `full_shaped_deserialize_project_drop` materially.
- `project_selection_set_*` disappears from flat profiles.
- `drop_in_place<Value>` disappears from flat profiles.
- object deserialization remains a cost, but no longer dominates together with projection and drop.

## Observability

Add debug counters/metrics during migration:

- flat pipeline used
- fallback to old pipeline
- fallback reason
- decoded bytes
- flat store value count
- flat store object field count
- flat store list item count
- merge count
- null propagation count

## Risks

- Null propagation can diverge subtly from current behavior.
- Fragment/type-condition behavior can diverge if compiled incorrectly.
- Multi-fetch flatten/entity behavior is significantly more complex than root fetch decoding.
- Per-fetch `FlatResponsePart` merging may introduce copies if not designed carefully.
- Plugin APIs may depend on `SubgraphResponse { data: Value }`.
- Extensions may need compatibility handling.
- Borrowed strings/raw JSON require careful byte retention across merged response parts.

## Design Constraints

- Preserve client response field order.
- Preserve current error and extension serialization behavior.
- Preserve custom scalar raw JSON behavior.
- Preserve current merge semantics unless explicitly changed with tests.
- Preserve existing public/plugin behavior or provide a migration layer.
- Keep old path available until flat path reaches full parity.

## Implementation Notes For LLMs

- Start by adding new modules rather than rewriting existing ones in place.
- Prefer small, testable units:
  - store allocation
  - store serialization
  - object decode
  - list decode
  - merge
  - flatten path resolution
- Do not remove existing shaped deserialization until the flat pipeline passes parity tests.
- Use existing `FieldProjectionPlan` and `FieldNullability` instead of inventing new GraphQL semantics.
- Use existing JSON writer helpers for scalar serialization.
- Reuse current `CustomScalarPaths` behavior exactly.
- Add semantic comparison tests against current pipeline before optimizing.

## Expected Outcome

The full flat pipeline should remove entire categories of work:

- no recursive `Value::Object` tree for response data
- no projection walk over `Value`
- no repeated object lookup during projection
- no recursive `Value` drop for response data
- less intermediate allocation

This is the architectural change needed to move beyond the small gains available from shaped deserialization micro-optimizations.
