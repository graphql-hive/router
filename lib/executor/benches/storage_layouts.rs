//! Three ways to store a parsed subgraph response, driven by the router's own machinery.
//!
//! The payload is `bench/expected_response.json`, the plan comes from running the real planner
//! over `bench/operation.graphql` against `bench/supergraph.graphql`, and every path and
//! program the benchmark walks is what that planner produced — the compiled `slot_path` of a
//! real `FlattenNode` and the compiled `requires` of the `FetchNode` under it. Nothing here
//! names a field or a slot by hand.
//!
//! ## The layouts
//!
//! - `slots`  — the router as it is. Runs the production code: `SubgraphResponse` parses into
//!   `Value` against the plan's own `ResponseShape`, `traverse_and_callback` walks the flatten
//!   path, `project_requires` executes the requires program, and `project_by_operation`
//!   serializes through the real `FieldProjectionPlan`.
//! - `stride` — a list of objects that share a shape becomes ONE run of `N * K` values;
//!   element `i`, slot `j` is `run[i * K + j]`, with no per-element pointer or allocation.
//! - `ids`    — Grafbase's shape: values carry ids into slabs, and objects keep `(key, value)`
//!   pairs found by linear scan, as `ResponseObject::find_by_response_key` does.
//!
//! `stride` and `ids` interpret the **same** `SlotPathSegment`, `RequiresStep` and
//! `FieldProjectionPlan` structures the router uses, so the only thing that differs between
//! the three is how the storage is read.
//!
//! ## The bias, stated up front
//!
//! The `slots` column runs production projection, which also handles `@skip`/`@include`
//! conditions, parent type guards, nullability and error propagation. The two ports handle
//! none of that — they are strictly less work. The comparison is therefore tilted *against*
//! the current design, which is the useful direction: if it still wins, the result holds.

use bumpalo::Bump;
use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use hive_router_plan_executor::introspection::schema::{PossibleTypes, SchemaWithMetadata};
use hive_router_plan_executor::json_writer::write_and_escape_string;
use hive_router_plan_executor::projection::plan::{FieldProjectionPlan, ProjectionValueSource};
use hive_router_plan_executor::projection::request::project_requires;
use hive_router_plan_executor::projection::response::project_by_operation;
use hive_router_plan_executor::response::subgraph_response::SubgraphResponse;
use hive_router_plan_executor::response::value::Value as SlotsValue;
use hive_router_plan_executor::utils::traverse::traverse_and_callback;
use hive_router_query_planner::ast::normalization::normalize_operation;
use hive_router_query_planner::graph::PlannerOverrideContext;
use hive_router_query_planner::planner::plan_nodes::{PlanNode, QueryPlan};
use hive_router_query_planner::planner::response_shape::ResponseShape;
use hive_router_query_planner::planner::slot_path::{RequiresStep, SlotPathSegment};
use hive_router_query_planner::planner::Planner;
use hive_router_query_planner::utils::cancellation::CancellationToken;
use hive_router_query_planner::utils::parsing::{parse_operation, parse_schema};
use sonic_rs::{JsonContainerTrait, JsonValueTrait, Value as Json};
use std::hint::black_box;

/// The first `Flatten { Fetch { requires } }` the planner produced: a real path to a real set
/// of entities, and the real program that turns each of them into a representation.
fn first_entity_fetch(plan: &QueryPlan) -> Option<(&[SlotPathSegment], &[RequiresStep])> {
    fn walk(node: &PlanNode) -> Option<(&[SlotPathSegment], &[RequiresStep])> {
        match node {
            PlanNode::Flatten(flatten) => {
                if let PlanNode::Fetch(fetch) = flatten.node.as_ref() {
                    if !fetch.compiled.requires.is_empty() {
                        return Some((&flatten.slot_path, &fetch.compiled.requires));
                    }
                }
                walk(&flatten.node)
            }
            PlanNode::Sequence(seq) => seq.nodes.iter().find_map(walk),
            PlanNode::Parallel(par) => par.nodes.iter().find_map(walk),
            _ => None,
        }
    }
    plan.node.as_ref().and_then(walk)
}

// ---------------------------------------------------------------------------------------
// Layout 2: a list of objects is one contiguous run of N * K values
// ---------------------------------------------------------------------------------------

enum StrideV<'a> {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(&'a str),
    Obj(&'a mut [StrideV<'a>]),
    List(&'a mut [StrideV<'a>]),
    ObjRun {
        values: &'a mut [StrideV<'a>],
        stride: usize,
    },
}

impl Default for StrideV<'_> {
    fn default() -> Self {
        StrideV::Null
    }
}

impl<'a> StrideV<'a> {
    fn as_str(&self) -> Option<&'a str> {
        match self {
            StrideV::Str(s) => Some(s),
            _ => None,
        }
    }
}

fn build_stride<'a>(json: &'a Json, shape: &ResponseShape, arena: &'a Bump) -> StrideV<'a> {
    if let Some(arr) = json.as_array() {
        if !arr.is_empty() && arr.iter().all(|v| v.is_object()) && !shape.fields.is_empty() {
            let stride = shape.fields.len();
            let values: &mut [StrideV<'a>] = arena.alloc_slice_fill_default(arr.len() * stride);
            for (i, item) in arr.iter().enumerate() {
                let obj = item.as_object().expect("checked");
                for (key, v) in obj.iter() {
                    if let Some(slot) = shape.fields.iter().position(|f| f.key == key) {
                        values[i * stride + slot] =
                            build_stride(v, &shape.fields[slot].shape, arena);
                    }
                }
            }
            return StrideV::ObjRun { values, stride };
        }
        let items = arena.alloc_slice_fill_with(arr.len(), |i| build_stride(&arr[i], shape, arena));
        return StrideV::List(items);
    }
    if let Some(obj) = json.as_object() {
        let slots: &mut [StrideV<'a>] = arena.alloc_slice_fill_default(shape.fields.len());
        for (key, v) in obj.iter() {
            if let Some(slot) = shape.fields.iter().position(|f| f.key == key) {
                slots[slot] = build_stride(v, &shape.fields[slot].shape, arena);
            }
        }
        return StrideV::Obj(slots);
    }
    if let Some(s) = json.as_str() {
        StrideV::Str(arena.alloc_str(s))
    } else if let Some(b) = json.as_bool() {
        StrideV::Bool(b)
    } else if let Some(n) = json.as_i64() {
        StrideV::Int(n)
    } else if let Some(n) = json.as_u64() {
        StrideV::Int(n as i64)
    } else if let Some(n) = json.as_f64() {
        StrideV::Float(n)
    } else {
        StrideV::Null
    }
}

/// `utils::traverse::traverse_and_callback`, ported to the stride layout.
///
/// The callback receives an object's **slots**, not a value: a run element is a window into
/// the run rather than a value of its own, so yielding slices is what the layout actually
/// offers. Fabricating a borrowed object per element would be inventing work the design does
/// not need — and could not be done safely from a shared borrow anyway.
fn traverse_stride<'a, 'v, F: FnMut(&'v [StrideV<'a>])>(
    current: &'v StrideV<'a>,
    path: &[SlotPathSegment],
    callback: &mut F,
) {
    let Some((segment, rest)) = path.split_first() else {
        match current {
            StrideV::List(items) => {
                for item in items.iter() {
                    if let StrideV::Obj(slots) = item {
                        callback(slots);
                    }
                }
            }
            StrideV::ObjRun { values, stride } => {
                for window in values.chunks_exact(*stride.max(&1)) {
                    callback(window);
                }
            }
            StrideV::Obj(slots) => callback(slots),
            _ => {}
        }
        return;
    };
    match segment {
        SlotPathSegment::List => match current {
            StrideV::List(items) => items.iter().for_each(|i| traverse_stride(i, rest, callback)),
            StrideV::ObjRun { .. } => traverse_stride(current, rest, callback),
            _ => {}
        },
        SlotPathSegment::Slot { slot, .. } => match current {
            StrideV::Obj(slots) => {
                if let Some(next) = slots.get(*slot) {
                    traverse_stride(next, rest, callback);
                }
            }
            StrideV::ObjRun { values, stride } => {
                for window in values.chunks_exact(*stride.max(&1)) {
                    if let Some(next) = window.get(*slot) {
                        traverse_stride(next, rest, callback);
                    }
                }
            }
            _ => {}
        },
        SlotPathSegment::TypenameEquals { .. } => traverse_stride(current, rest, callback),
    }
}

/// `projection::request::project_requires`, ported to the stride layout.
///
/// Same structure as production: one recursive step walker, so a type condition emits through
/// exactly the same leaf path as a top-level field rather than a second copy of it. The first
/// version of this had a duplicated emitter inside `OnType` that only handled strings, and
/// silently nulled every integer — the equality check against production caught it.
fn requires_stride(steps: &[RequiresStep], entity: &[StrideV<'_>], out: &mut Vec<u8>) {
    out.push(b'{');
    let mut first = true;
    emit_requires_steps(steps, entity, out, &mut first);
    out.push(b'}');
}

fn emit_requires_steps(
    steps: &[RequiresStep],
    entity: &[StrideV<'_>],
    out: &mut Vec<u8>,
    first: &mut bool,
) {
    for step in steps {
        match step {
            // Production writes `__typename` from the reserved slot rather than as a step.
            RequiresStep::Leaf { key, .. } if key == "__typename" => continue,
            RequiresStep::Leaf { key, slot } => {
                let Some(value) = entity.get(*slot) else { continue };
                if !*first {
                    out.push(b',');
                }
                *first = false;
                write_key(out, key);
                write_scalar(value, out);
            }
            RequiresStep::Enter { key, slot, steps } => {
                let Some(StrideV::Obj(child)) = entity.get(*slot) else {
                    continue;
                };
                if !*first {
                    out.push(b',');
                }
                *first = false;
                write_key(out, key);
                requires_stride(steps, child, out);
            }
            RequiresStep::OnType {
                typename_slot,
                type_condition,
                steps,
            } => {
                let matches = typename_slot
                    .and_then(|s| entity.get(s))
                    .and_then(|v| v.as_str())
                    .map(|t| t == type_condition)
                    .unwrap_or(true);
                if matches {
                    emit_requires_steps(steps, entity, out, first);
                }
            }
        }
    }
}

fn write_scalar(value: &StrideV<'_>, out: &mut Vec<u8>) {
    match value {
        StrideV::Str(s) => write_str(out, s),
        StrideV::Int(n) => out.extend_from_slice(itoa::Buffer::new().format(*n).as_bytes()),
        StrideV::Float(n) => out.extend_from_slice(ryu::Buffer::new().format(*n).as_bytes()),
        StrideV::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        _ => out.extend_from_slice(b"null"),
    }
}

/// `projection::response::project_by_operation`, ported: driven by the same plan, minus the
/// conditions, guards and propagation it does not model.
fn project_stride(plans: &[FieldProjectionPlan], value: &StrideV<'_>, out: &mut Vec<u8>) {
    match value {
        StrideV::List(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                project_stride(plans, item, out);
            }
            out.push(b']');
        }
        StrideV::ObjRun { values, stride } => {
            out.push(b'[');
            let n = values.len() / stride.max(&1);
            for i in 0..n {
                if i > 0 {
                    out.push(b',');
                }
                project_run_object(plans, values, *stride, i, out);
            }
            out.push(b']');
        }
        StrideV::Obj(slots) => project_object(plans, slots, out),
        StrideV::Str(s) => write_str(out, s),
        StrideV::Int(n) => out.extend_from_slice(itoa::Buffer::new().format(*n).as_bytes()),
        StrideV::Float(n) => out.extend_from_slice(ryu::Buffer::new().format(*n).as_bytes()),
        StrideV::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        StrideV::Null => out.extend_from_slice(b"null"),
    }
}

fn project_object(plans: &[FieldProjectionPlan], slots: &[StrideV<'_>], out: &mut Vec<u8>) {
    out.push(b'{');
    for (i, plan) in plans.iter().enumerate() {
        if i > 0 {
            out.push(b',');
        }
        out.extend_from_slice(&plan.response_key_json);
        emit_plan_value(plan, slots.get(plan.slot), out);
    }
    out.push(b'}');
}

fn project_run_object(
    plans: &[FieldProjectionPlan],
    values: &[StrideV<'_>],
    stride: usize,
    index: usize,
    out: &mut Vec<u8>,
) {
    out.push(b'{');
    for (i, plan) in plans.iter().enumerate() {
        if i > 0 {
            out.push(b',');
        }
        out.extend_from_slice(&plan.response_key_json);
        emit_plan_value(plan, values.get(index * stride + plan.slot), out);
    }
    out.push(b'}');
}

fn emit_plan_value(plan: &FieldProjectionPlan, value: Option<&StrideV<'_>>, out: &mut Vec<u8>) {
    match (&plan.value, value) {
        (ProjectionValueSource::Null, _) | (_, None) => out.extend_from_slice(b"null"),
        (ProjectionValueSource::ResponseData { selections }, Some(v)) => match selections {
            Some(children) => project_stride(children, v, out),
            None => project_stride(&[], v, out),
        },
    }
}

/// The same simplified reader as the stride port, over the *current* layout.
///
/// Without this the comparison is production-versus-toy: the production column also resolves
/// conditions, type guards, nullability and error propagation, none of which the ports model.
/// This column strips the current layout down to exactly what the ports do, so a difference
/// between it and `stride` is a difference in storage and nothing else.
fn requires_slots_simple(steps: &[RequiresStep], entity: &SlotsValue<'_>, out: &mut Vec<u8>) {
    out.push(b'{');
    let mut first = true;
    emit_slots_steps(steps, entity, out, &mut first);
    out.push(b'}');
}

fn emit_slots_steps(
    steps: &[RequiresStep],
    entity: &SlotsValue<'_>,
    out: &mut Vec<u8>,
    first: &mut bool,
) {
    for step in steps {
        match step {
            RequiresStep::Leaf { key, .. } if key == "__typename" => continue,
            RequiresStep::Leaf { key, slot } => {
                let Some(value) = entity.slot(*slot) else { continue };
                if !*first {
                    out.push(b',');
                }
                *first = false;
                write_key(out, key);
                write_slots_scalar(value, out);
            }
            RequiresStep::Enter { key, slot, steps } => {
                let Some(child) = entity.slot(*slot) else { continue };
                if !*first {
                    out.push(b',');
                }
                *first = false;
                write_key(out, key);
                requires_slots_simple(steps, child, out);
            }
            RequiresStep::OnType {
                typename_slot,
                type_condition,
                steps,
            } => {
                let matches = typename_slot
                    .and_then(|s| entity.slot(s))
                    .and_then(SlotsValue::as_str)
                    .map(|t| t == type_condition)
                    .unwrap_or(true);
                if matches {
                    emit_slots_steps(steps, entity, out, first);
                }
            }
        }
    }
}

fn write_slots_scalar(value: &SlotsValue<'_>, out: &mut Vec<u8>) {
    match value {
        SlotsValue::String(s) => write_str(out, s),
        // A custom scalar: the subgraph's own bytes, copied straight through.
        SlotsValue::RawJson(raw) => out.extend_from_slice(raw.as_bytes()),
        SlotsValue::I64(n) => out.extend_from_slice(itoa::Buffer::new().format(*n).as_bytes()),
        SlotsValue::U64(n) => out.extend_from_slice(itoa::Buffer::new().format(*n).as_bytes()),
        SlotsValue::F64(n) => out.extend_from_slice(ryu::Buffer::new().format(*n).as_bytes()),
        SlotsValue::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        _ => out.extend_from_slice(b"null"),
    }
}

/// `project_by_operation`'s shape, simplified to match the ports.
fn project_slots_simple(plans: &[FieldProjectionPlan], value: &SlotsValue<'_>, out: &mut Vec<u8>) {
    match value {
        SlotsValue::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                project_slots_simple(plans, item, out);
            }
            out.push(b']');
        }
        SlotsValue::Object(slots) => {
            out.push(b'{');
            for (i, plan) in plans.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                out.extend_from_slice(&plan.response_key_json);
                match (&plan.value, slots.get(plan.slot)) {
                    (ProjectionValueSource::Null, _) | (_, None) => {
                        out.extend_from_slice(b"null")
                    }
                    (ProjectionValueSource::ResponseData { selections }, Some(v)) => {
                        project_slots_simple(selections.as_deref().map_or(&[], |s| s), v, out)
                    }
                }
            }
            out.push(b'}');
        }
        other => write_slots_scalar(other, out),
    }
}

// ---------------------------------------------------------------------------------------
// Layout 3: ids into slabs, objects keyed and scanned (Grafbase's `ResponseObject`)
//
// Driven by the same compiled programs: `SlotPathSegment::Slot` and `RequiresStep::Leaf` both
// carry the response key alongside the slot, and `FieldProjectionPlan` carries it too, so a
// keyed layout runs the identical plan — it just looks fields up by name instead of index.
// ---------------------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum IdV<'a> {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(&'a str),
    Obj(u32),
    List(u32),
}

#[derive(Default)]
struct Slab<'a> {
    objects: Vec<Vec<(&'a str, IdV<'a>)>>,
    lists: Vec<Vec<IdV<'a>>>,
}

impl<'a> Slab<'a> {
    /// `ResponseObject::find_by_response_key`: a linear scan over the object's fields.
    #[inline]
    fn get(&self, object: u32, key: &str) -> IdV<'a> {
        self.objects[object as usize]
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| *v)
            .unwrap_or(IdV::Null)
    }
}

fn build_ids<'a>(json: &'a Json, slab: &mut Slab<'a>) -> IdV<'a> {
    if let Some(arr) = json.as_array() {
        let items: Vec<IdV<'a>> = arr.iter().map(|v| build_ids(v, slab)).collect();
        slab.lists.push(items);
        return IdV::List((slab.lists.len() - 1) as u32);
    }
    if let Some(obj) = json.as_object() {
        let mut fields = Vec::with_capacity(obj.iter().count());
        for (key, v) in obj.iter() {
            let value = build_ids(v, slab);
            fields.push((key, value));
        }
        slab.objects.push(fields);
        return IdV::Obj((slab.objects.len() - 1) as u32);
    }
    if let Some(x) = json.as_str() {
        IdV::Str(x)
    } else if let Some(b) = json.as_bool() {
        IdV::Bool(b)
    } else if let Some(n) = json.as_i64() {
        IdV::Int(n)
    } else if let Some(n) = json.as_u64() {
        IdV::Int(n as i64)
    } else if let Some(n) = json.as_f64() {
        IdV::Float(n)
    } else {
        IdV::Null
    }
}

fn traverse_ids<F: FnMut(u32)>(current: IdV<'_>, path: &[SlotPathSegment], slab: &Slab<'_>, cb: &mut F) {
    let Some((segment, rest)) = path.split_first() else {
        match current {
            IdV::List(id) => {
                for item in &slab.lists[id as usize] {
                    if let IdV::Obj(o) = item {
                        cb(*o);
                    }
                }
            }
            IdV::Obj(o) => cb(o),
            _ => {}
        }
        return;
    };
    match segment {
        SlotPathSegment::List => {
            if let IdV::List(id) = current {
                for item in &slab.lists[id as usize] {
                    traverse_ids(*item, rest, slab, cb);
                }
            }
        }
        SlotPathSegment::Slot { key, .. } => {
            if let IdV::Obj(o) = current {
                traverse_ids(slab.get(o, key), rest, slab, cb);
            }
        }
        SlotPathSegment::TypenameEquals { .. } => traverse_ids(current, rest, slab, cb),
    }
}

fn requires_ids(steps: &[RequiresStep], object: u32, slab: &Slab<'_>, out: &mut Vec<u8>) {
    out.push(b'{');
    let mut first = true;
    emit_ids_steps(steps, object, slab, out, &mut first);
    out.push(b'}');
}

fn emit_ids_steps(
    steps: &[RequiresStep],
    object: u32,
    slab: &Slab<'_>,
    out: &mut Vec<u8>,
    first: &mut bool,
) {
    for step in steps {
        match step {
            RequiresStep::Leaf { key, .. } if key == "__typename" => continue,
            RequiresStep::Leaf { key, .. } => {
                let value = slab.get(object, key);
                if matches!(value, IdV::Null) {
                    continue;
                }
                if !*first {
                    out.push(b',');
                }
                *first = false;
                write_key(out, key);
                write_ids_scalar(value, out);
            }
            RequiresStep::Enter { key, steps, .. } => {
                let IdV::Obj(child) = slab.get(object, key) else {
                    continue;
                };
                if !*first {
                    out.push(b',');
                }
                *first = false;
                write_key(out, key);
                requires_ids(steps, child, slab, out);
            }
            RequiresStep::OnType {
                type_condition,
                steps,
                ..
            } => {
                let matches = match slab.get(object, "__typename") {
                    IdV::Str(t) => t == type_condition,
                    _ => true,
                };
                if matches {
                    emit_ids_steps(steps, object, slab, out, first);
                }
            }
        }
    }
}

fn write_ids_scalar(value: IdV<'_>, out: &mut Vec<u8>) {
    match value {
        IdV::Str(s) => write_str(out, s),
        IdV::Int(n) => out.extend_from_slice(itoa::Buffer::new().format(n).as_bytes()),
        IdV::Float(n) => out.extend_from_slice(ryu::Buffer::new().format(n).as_bytes()),
        IdV::Bool(b) => out.extend_from_slice(if b { b"true" } else { b"false" }),
        _ => out.extend_from_slice(b"null"),
    }
}

fn project_ids(plans: &[FieldProjectionPlan], value: IdV<'_>, slab: &Slab<'_>, out: &mut Vec<u8>) {
    match value {
        IdV::List(id) => {
            out.push(b'[');
            for (i, item) in slab.lists[id as usize].iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                project_ids(plans, *item, slab, out);
            }
            out.push(b']');
        }
        IdV::Obj(o) if plans.is_empty() => {
            // A custom scalar: the plan stops here but the value is arbitrary JSON. Grafbase
            // keeps it materialised (`maps: Vec<Vec<(String, ResponseValue)>>`) and walks it,
            // where we hold the subgraph's bytes and memcpy them.
            out.push(b'{');
            for (i, (key, value)) in slab.objects[o as usize].iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_key(out, key);
                project_ids(&[], *value, slab, out);
            }
            out.push(b'}');
        }
        IdV::Obj(o) => {
            out.push(b'{');
            for (i, plan) in plans.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                out.extend_from_slice(&plan.response_key_json);
                match &plan.value {
                    ProjectionValueSource::Null => out.extend_from_slice(b"null"),
                    ProjectionValueSource::ResponseData { selections } => project_ids(
                        selections.as_deref().map_or(&[], |s| s),
                        slab.get(o, &plan.response_key),
                        slab,
                        out,
                    ),
                }
            }
            out.push(b'}');
        }
        other => write_ids_scalar(other, out),
    }
}

// ---------------------------------------------------------------------------------------
// Layout 4: Grafbase's *actual* client-response path — no plan at all
//
// `crates/engine/src/response/read/ser/data.rs` serializes with a context of
// `{ keys, data, schema }` and nothing else. Every field carries a `PositionedResponseKey`,
// fields are stored sorted so unrequested ones (`query_position: None`) come first, and the
// serializer skips forward to the first positioned field then emits the rest unconditionally:
//
//     for ResponseObjectField { key, value, .. } in fields.by_ref() {
//         if key.query_position.is_some() { ...emit...; for rest { ...emit... } break; }
//     }
//
// So order, client-visibility and the response key all live in the data, decided once when the
// tree was written. Modelled here with the injected `__typename` every entity position carries
// in production, so the skip has something to skip.
// ---------------------------------------------------------------------------------------

struct PosField<'a> {
    key: &'a str,
    /// `None` for a field the client did not ask for; sorted to the front, as they are there.
    query_position: Option<u32>,
    value: PosV<'a>,
}

#[derive(Clone, Copy)]
enum PosV<'a> {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(&'a str),
    Obj(u32),
    List(u32),
}

#[derive(Default)]
struct PosSlab<'a> {
    objects: Vec<Vec<PosField<'a>>>,
    lists: Vec<Vec<PosV<'a>>>,
}

fn build_positional<'a>(
    json: &'a Json,
    plans: &'a [FieldProjectionPlan],
    slab: &mut PosSlab<'a>,
) -> PosV<'a> {
    if let Some(arr) = json.as_array() {
        let items: Vec<PosV<'a>> = arr
            .iter()
            .map(|v| build_positional(v, plans, slab))
            .collect();
        slab.lists.push(items);
        return PosV::List((slab.lists.len() - 1) as u32);
    }
    if let Some(obj) = json.as_object() {
        let mut fields: Vec<PosField<'a>> = Vec::with_capacity(plans.len() + 1);
        // The planner-injected `__typename` every entity position carries: present in the
        // tree, never requested, so it sorts first and the serializer walks past it.
        fields.push(PosField {
            key: "__typename",
            query_position: None,
            value: PosV::Null,
        });
        for (position, plan) in plans.iter().enumerate() {
            let Some(v) = obj.get(&plan.response_key) else {
                continue;
            };
            let children: &'a [FieldProjectionPlan] = match &plan.value {
                ProjectionValueSource::ResponseData { selections } => {
                    selections.as_deref().map_or(&[][..], |s| s)
                }
                ProjectionValueSource::Null => &[][..],
            };
            let value = if children.is_empty() && (v.is_object() || v.is_array()) {
                build_positional_opaque(v, slab)
            } else {
                build_positional(v, children, slab)
            };
            fields.push(PosField {
                key: &plan.response_key,
                query_position: Some(position as u32),
                value,
            });
        }
        // Stored sorted by position, unpositioned first — the invariant the serializer relies on.
        fields.sort_by_key(|f| f.query_position);
        slab.objects.push(fields);
        return PosV::Obj((slab.objects.len() - 1) as u32);
    }
    if let Some(x) = json.as_str() {
        PosV::Str(x)
    } else if let Some(b) = json.as_bool() {
        PosV::Bool(b)
    } else if let Some(n) = json.as_i64() {
        PosV::Int(n)
    } else if let Some(n) = json.as_u64() {
        PosV::Int(n as i64)
    } else if let Some(n) = json.as_f64() {
        PosV::Float(n)
    } else {
        PosV::Null
    }
}

/// Arbitrary JSON under a custom scalar, materialised the way a design without passthrough
/// must: every object becomes a field vector, every key a stored key, every value a node.
fn build_positional_opaque<'a>(json: &'a Json, slab: &mut PosSlab<'a>) -> PosV<'a> {
    if let Some(arr) = json.as_array() {
        let items: Vec<PosV<'a>> = arr.iter().map(|v| build_positional_opaque(v, slab)).collect();
        slab.lists.push(items);
        return PosV::List((slab.lists.len() - 1) as u32);
    }
    if let Some(obj) = json.as_object() {
        let mut fields = Vec::new();
        for (key, v) in obj.iter() {
            let value = build_positional_opaque(v, slab);
            // Every field is emitted, so they all count as positioned.
            fields.push(PosField { key, query_position: Some(fields.len() as u32), value });
        }
        slab.objects.push(fields);
        return PosV::Obj((slab.objects.len() - 1) as u32);
    }
    if let Some(x) = json.as_str() {
        PosV::Str(x)
    } else if let Some(b) = json.as_bool() {
        PosV::Bool(b)
    } else if let Some(n) = json.as_i64() {
        PosV::Int(n)
    } else if let Some(n) = json.as_u64() {
        PosV::Int(n as i64)
    } else if let Some(n) = json.as_f64() {
        PosV::Float(n)
    } else {
        PosV::Null
    }
}

/// Their serializer: no plan, no lookup, no per-field decision beyond the skip.
fn serialize_positional(value: PosV<'_>, slab: &PosSlab<'_>, out: &mut Vec<u8>) {
    match value {
        PosV::List(id) => {
            out.push(b'[');
            for (i, item) in slab.lists[id as usize].iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                serialize_positional(*item, slab, out);
            }
            out.push(b']');
        }
        PosV::Obj(id) => {
            out.push(b'{');
            let mut fields = slab.objects[id as usize].iter();
            let mut first = true;
            for field in fields.by_ref() {
                if field.query_position.is_some() {
                    write_key(out, field.key);
                    serialize_positional(field.value, slab, out);
                    first = false;
                    for field in fields.by_ref() {
                        out.push(b',');
                        write_key(out, field.key);
                        serialize_positional(field.value, slab, out);
                    }
                    break;
                }
            }
            let _ = first;
            out.push(b'}');
        }
        PosV::Str(s) => write_str(out, s),
        PosV::Int(n) => out.extend_from_slice(itoa::Buffer::new().format(n).as_bytes()),
        PosV::Float(n) => out.extend_from_slice(ryu::Buffer::new().format(n).as_bytes()),
        PosV::Bool(b) => out.extend_from_slice(if b { b"true" } else { b"false" }),
        PosV::Null => out.extend_from_slice(b"null"),
    }
}

/// Byte-equality with a readable failure: these outputs are megabytes, so print the first
/// divergence and a window around it rather than the whole thing.
fn assert_same_bytes(left: &[u8], right: &[u8], left_name: &str, right_name: &str) {
    if left == right {
        return;
    }
    let at = left
        .iter()
        .zip(right.iter())
        .position(|(a, b)| a != b)
        .unwrap_or(left.len().min(right.len()));
    let from = at.saturating_sub(80);
    panic!(
        "{left_name} and {right_name} diverge at byte {at} (len {} vs {})\n  {left_name}: ...{}\n  {right_name}: ...{}",
        left.len(),
        right.len(),
        String::from_utf8_lossy(&left[from..(at + 80).min(left.len())]),
        String::from_utf8_lossy(&right[from..(at + 80).min(right.len())]),
    );
}

/// The same shape with every passthrough turned off, so a payload can be measured both ways.
fn shape_has_raw(shape: &ResponseShape) -> bool {
    shape.raw || shape.fields.iter().any(|f| shape_has_raw(&f.shape))
}

fn strip_raw(shape: &ResponseShape) -> ResponseShape {
    let fields = shape
        .fields
        .iter()
        .map(|f| hive_router_query_planner::planner::response_shape::ResponseShapeField {
            key: f.key.clone(),
            shape: strip_raw(&f.shape),
        })
        .collect::<Vec<_>>();
    let inert = fields.iter().all(|f| f.shape.inert);
    ResponseShape {
        fields,
        raw: false,
        inert,
    }
}

fn write_key(out: &mut Vec<u8>, key: &str) {
    out.push(b'"');
    out.extend_from_slice(key.as_bytes());
    out.extend_from_slice(br#"":"#);
}

/// The same writer production uses.
///
/// The first version of this benchmark copied string bytes raw. The equality check still
/// passed, because this payload contains nothing that needs escaping — so the "same effort"
/// reader was quietly skipping the single hottest line in projection
/// (`write_and_escape_string`, 1.36% of on-CPU) and the comparison was measuring that, not
/// the storage layout.
fn write_str(out: &mut Vec<u8>, s: &str) {
    write_and_escape_string(out, s);
}

fn storage_layouts(c: &mut Criterion) {
    let bench = concat!(env!("CARGO_MANIFEST_DIR"), "/../../bench/");
    run_fixture(
        c,
        "federated",
        &format!("{bench}supergraph.graphql"),
        &format!("{bench}operation.graphql"),
        &format!("{bench}expected_response.json"),
    );

    // A second fixture with a very different shape: one subgraph, no entity resolution, and a
    // 3 MB deeply-nested payload. The federated one is 87 KB across seven levels with small
    // lists; conclusions drawn from either alone would not obviously carry to the other.
    let ct = concat!(env!("CARGO_MANIFEST_DIR"), "/../../");
    let (sdl, query, payload) = (
        format!("{ct}ct-super.graphql"),
        format!("{ct}ct-query.graphql"),
        format!("{ct}ct-payload.json"),
    );
    if std::path::Path::new(&payload).exists() {
        run_fixture(c, "json_heavy", &sdl, &query, &payload);
    } else {
        eprintln!("skipping json_heavy fixture: {payload} not found");
    }
}

fn run_fixture(
    c: &mut Criterion,
    fixture: &str,
    sdl_path: &str,
    query_path: &str,
    payload_path: &str,
) {
    let sdl = std::fs::read_to_string(sdl_path).expect("supergraph");
    let operation_src = std::fs::read_to_string(query_path).expect("op");
    let raw = std::fs::read_to_string(payload_path).expect("response");

    let schema = parse_schema(&sdl);
    let planner = Planner::new_from_supergraph(&schema, Default::default()).expect("planner");
    let document = parse_operation(&operation_src);
    let normalized =
        normalize_operation(&planner.supergraph, &document, None).expect("normalize");
    let operation = normalized.executable_operation();
    let metadata = planner.consumer_schema.schema_metadata();
    let query_plan = planner
        .plan_from_normalized_operation(
            operation,
            PlannerOverrideContext::default(),
            &CancellationToken::default(),
        )
        .expect("plan");
    let shape = &query_plan.response_shape;
    let (root_type_name, projection_plan) = FieldProjectionPlan::from_operation(operation, &metadata);
    // A single-subgraph query has no entity fetch at all, so the requires stages are skipped
    // rather than measured against an empty program.
    let (entity_path, requires): (&[SlotPathSegment], &[RequiresStep]) =
        first_entity_fetch(&query_plan).unwrap_or((&[], &[]));
    let has_entities = !requires.is_empty();

    let envelope: Json = sonic_rs::from_str(&raw).expect("json");
    let data_json = sonic_rs::to_string(envelope.get("data").expect("data")).expect("data json");
    let bytes = raw.len() as u64;
    let size_hint = raw.len() * 12 / 10;

    let owned = SubgraphResponse::parse_data_with_shape(&data_json, shape);
    let slots_data = &owned.data;
    let arena = Bump::new();
    let stride_json: Json = sonic_rs::from_str(&data_json).expect("json");
    let stride_data = build_stride(&stride_json, shape, &arena);
    let ids_json: Json = sonic_rs::from_str(&data_json).expect("json");
    let mut slab = Slab::default();
    let ids_data = build_ids(&ids_json, &mut slab);
    let pos_json: Json = sonic_rs::from_str(&data_json).expect("json");
    let mut pos_slab = PosSlab::default();
    let pos_data = build_positional(&pos_json, &projection_plan, &mut pos_slab);

    // Same answer, or the comparison is meaningless.
    let possible_types: &PossibleTypes = &metadata.possible_types;
    if has_entities {
        let mut a = Vec::new();
        traverse_and_callback(slots_data, entity_path, possible_types, &mut |e| {
            project_requires(possible_types, requires, e, &mut a, true, None);
        });
        let mut b = Vec::new();
        traverse_stride(&stride_data, entity_path, &mut |e| {
            requires_stride(requires, e, &mut b)
        });
        assert!(!a.is_empty(), "the planner's flatten path found no entities");
        let mut c_same = Vec::new();
        traverse_and_callback(slots_data, entity_path, possible_types, &mut |e| {
            requires_slots_simple(requires, e, &mut c_same);
        });
        assert_eq!(
            String::from_utf8_lossy(&a),
            String::from_utf8_lossy(&b),
            "stride requires differed from production"
        );
        assert_eq!(
            String::from_utf8_lossy(&a),
            String::from_utf8_lossy(&c_same),
            "the equal-effort slots reader differed from production"
        );
        let mut d = Vec::new();
        traverse_ids(ids_data, entity_path, &slab, &mut |o| {
            requires_ids(requires, o, &slab, &mut d)
        });
        assert_eq!(
            String::from_utf8_lossy(&a),
            String::from_utf8_lossy(&d),
            "ids requires differed from production"
        );

    }

    // The plan-free serializer must emit exactly what the plan-driven one does: same keys,
    // same order, the injected `__typename` skipped.
    let mut plan_driven = Vec::new();
    project_slots_simple(&projection_plan, slots_data, &mut plan_driven);
    let mut positional = Vec::new();
    serialize_positional(pos_data, &pos_slab, &mut positional);
    // The alternative layouts have no passthrough: Grafbase materialises arbitrary JSON into
    // `maps: Vec<Vec<(String, ResponseValue)>>` and has no raw variant at all, and neither of
    // my ports models one. A payload with custom scalars therefore cannot be compared across
    // layouts here — the ports would be re-serialising what we memcpy — so the layout groups
    // are skipped and the passthrough itself is priced instead.
    let has_passthrough = shape_has_raw(shape);

    // Production too, so a divergence tells us whether the fault is in the port or in the
    // slots the real projection plan resolved.
    let production = project_by_operation(
        slots_data,
        vec![],
        &Default::default(),
        root_type_name,
        &projection_plan,
        &None,
        size_hint,
        &metadata,
    )
    .expect("projection");
    // Production wraps the tree: `{"data":<tree>}` when there are no errors.
    const ENVELOPE: &[u8] = br#"{"data":"#;
    assert!(production.starts_with(ENVELOPE) && production.ends_with(b"}"));
    let production_data = &production[ENVELOPE.len()..production.len() - 1];
    assert_same_bytes(production_data, &positional, "production", "plan-free");
    assert_same_bytes(production_data, &plan_driven, "production", "plan-driven");
    let mut ids_out = Vec::new();
    project_ids(&projection_plan, ids_data, &slab, &mut ids_out);
    assert_same_bytes(production_data, &ids_out, "production", "ids");

    // ------------------------------------------------------------------------------
    // What each production pass over the same data costs.
    //
    // The layout benchmarks above ask where values should live. This asks something else:
    // how much of the work is *re-walking* a tree we have already walked. Fusing passes —
    // writing straight into the merged tree as bytes arrive, collecting entity positions and
    // emitting representations on the way — can only ever recover what the separate passes
    // cost, so this is the ceiling on that idea.
    // ------------------------------------------------------------------------------
    let mut group = c.benchmark_group(format!("{fixture}/pipeline"));
    group.throughput(Throughput::Bytes(bytes));
    group.bench_function("1_parse", |b| {
        b.iter(|| black_box(SubgraphResponse::parse_data_with_shape(&data_json, shape).data.is_object()))
    });
    group.bench_function("2_merge_into_empty", |b| {
        b.iter_batched(
            || {
                (
                    SubgraphResponse::parse_data_with_shape(&data_json, shape),
                    SubgraphResponse::parse_data_with_shape(&data_json, shape),
                )
            },
            |(mut target, source)| {
                hive_router_plan_executor::response::merge::deep_merge(&mut target.data, source.data);
                black_box(target.data.is_object())
            },
            BatchSize::SmallInput,
        )
    });
    if has_entities {
    group.bench_function("3_traverse_only", |b| {
        b.iter(|| {
            let mut n = 0usize;
            traverse_and_callback(slots_data, entity_path, possible_types, &mut |_| n += 1);
            black_box(n)
        })
    });
    group.bench_function("4_traverse_and_requires", |b| {
        b.iter_batched_ref(
            || Vec::<u8>::with_capacity(16 * 1024),
            |out| {
                traverse_and_callback(slots_data, entity_path, possible_types, &mut |e| {
                    project_requires(possible_types, requires, e, out, true, None);
                });
                black_box(out.len())
            },
            BatchSize::SmallInput,
        )
    });
    }
    if has_passthrough {
        let structured = strip_raw(shape);
        let structured_owned = SubgraphResponse::parse_data_with_shape(&data_json, &structured);
        // Turning passthrough off is only a fair comparison if it still produces the response.
        // A custom scalar's value is arbitrary JSON and a leaf slot has nowhere to put an
        // object, so stripping `raw` there silently drops content -- which would make the
        // "no passthrough" column measure producing a smaller, wrong answer.
        let with = project_by_operation(slots_data, vec![], &Default::default(), root_type_name,
            &projection_plan, &None, size_hint, &metadata).expect("projection");
        let without = project_by_operation(&structured_owned.data, vec![], &Default::default(),
            root_type_name, &projection_plan, &None, size_hint, &metadata).expect("projection");
        // How much of the payload never gets parsed at all.
        fn raw_bytes(v: &hive_router_plan_executor::response::value::Value<'_>, n: &mut usize, count: &mut usize) {
            use hive_router_plan_executor::response::value::Value as V;
            match v {
                V::RawJson(s) => {
                    *n += s.len();
                    *count += 1;
                }
                V::Array(items) => items.iter().for_each(|i| raw_bytes(i, n, count)),
                V::Object(slots) => slots.iter().for_each(|i| raw_bytes(i, n, count)),
                _ => {}
            }
        }
        let (mut n, mut count) = (0usize, 0usize);
        raw_bytes(slots_data, &mut n, &mut count);
        eprintln!(
            "[{fixture}] passthrough covers {n} bytes in {count} values = {:.1}% of the payload",
            100.0 * n as f64 / raw.len() as f64
        );
        // What a design without passthrough has to materialise for those same values.
        // Grafbase keeps arbitrary JSON as `maps: Vec<Vec<(String, ResponseValue)>>`, so every
        // key inside one is an owned `String` allocation and every object is a `Vec`.
        fn count_inside(v: &hive_router_plan_executor::response::value::Value<'_>, objs: &mut usize, keys: &mut usize, vals: &mut usize) {
            use hive_router_plan_executor::response::value::Value as V;
            fn walk_json(j: &Json, objs: &mut usize, keys: &mut usize, vals: &mut usize) {
                *vals += 1;
                if let Some(o) = j.as_object() {
                    *objs += 1;
                    for (_, child) in o.iter() {
                        *keys += 1;
                        walk_json(child, objs, keys, vals);
                    }
                } else if let Some(a) = j.as_array() {
                    for child in a.iter() {
                        walk_json(child, objs, keys, vals);
                    }
                }
            }
            match v {
                V::RawJson(s) => {
                    if let Ok(j) = sonic_rs::from_str::<Json>(s) {
                        walk_json(&j, objs, keys, vals);
                    }
                }
                V::Array(items) => items.iter().for_each(|i| count_inside(i, objs, keys, vals)),
                V::Object(slots) => slots.iter().for_each(|i| count_inside(i, objs, keys, vals)),
                _ => {}
            }
        }
        let (mut objs, mut keys, mut vals) = (0usize, 0usize, 0usize);
        count_inside(slots_data, &mut objs, &mut keys, &mut vals);
        eprintln!(
            "[{fixture}] inside those: {objs} objects, {keys} keys, {vals} values -- \
             all of which a materialising design allocates per request"
        );
        eprintln!(
            "[{fixture}] passthrough on: {} bytes, off: {} bytes ({:+.1}%)",
            with.len(),
            without.len(),
            100.0 * (without.len() as f64 / with.len() as f64 - 1.0)
        );
        group.bench_function("1_parse_no_passthrough", |b| {
            b.iter(|| {
                black_box(
                    SubgraphResponse::parse_data_with_shape(&data_json, &structured)
                        .data
                        .is_object(),
                )
            })
        });
        group.bench_function("5_project_no_passthrough", |b| {
            b.iter(|| {
                let out = project_by_operation(
                    &structured_owned.data,
                    vec![],
                    &Default::default(),
                    root_type_name,
                    &projection_plan,
                    &None,
                    size_hint,
                    &metadata,
                )
                .expect("projection");
                black_box(out.len())
            })
        });
    }
    group.bench_function("5_project_response", |b| {
        b.iter(|| {
            let out = project_by_operation(
                slots_data,
                vec![],
                &Default::default(),
                root_type_name,
                &projection_plan,
                &None,
                size_hint,
                &metadata,
            )
            .expect("projection");
            black_box(out.len())
        })
    });
    group.finish();

    if !has_passthrough {
    let mut group = c.benchmark_group(format!("{fixture}/storage/build"));
    group.throughput(Throughput::Bytes(bytes));
    group.bench_function("slots", |b| {
        b.iter(|| black_box(SubgraphResponse::parse_data_with_shape(&data_json, shape).data.is_object()))
    });
    group.bench_function("stride", |b| {
        b.iter_batched(
            Bump::new,
            |arena| {
                let json: Json = sonic_rs::from_str(&data_json).expect("json");
                black_box(matches!(build_stride(&json, shape, &arena), StrideV::Obj(_)))
            },
            BatchSize::SmallInput,
        )
    });
    group.finish();
    }

    if has_entities {
        let mut group = c.benchmark_group(format!("{fixture}/storage/requires"));
        group.throughput(Throughput::Bytes(bytes));
        group.bench_function("slots", |b| {
            b.iter_batched_ref(
                || Vec::<u8>::with_capacity(16 * 1024),
                |out| {
                    traverse_and_callback(slots_data, entity_path, possible_types, &mut |e| {
                        project_requires(possible_types, requires, e, out, true, None);
                    });
                    black_box(out.len())
                },
                BatchSize::SmallInput,
            )
        });
        group.bench_function("slots_same_effort", |b| {
            b.iter_batched_ref(
                || Vec::<u8>::with_capacity(16 * 1024),
                |out| {
                    traverse_and_callback(slots_data, entity_path, possible_types, &mut |e| {
                        requires_slots_simple(requires, e, out);
                    });
                    black_box(out.len())
                },
                BatchSize::SmallInput,
            )
        });
        group.bench_function("stride", |b| {
            b.iter_batched_ref(
                || Vec::<u8>::with_capacity(16 * 1024),
                |out| {
                    traverse_stride(&stride_data, entity_path, &mut |e| {
                        requires_stride(requires, e, out)
                    });
                    black_box(out.len())
                },
                BatchSize::SmallInput,
            )
        });
        group.bench_function("ids", |b| {
            b.iter_batched_ref(
                || Vec::<u8>::with_capacity(16 * 1024),
                |out| {
                    traverse_ids(ids_data, entity_path, &slab, &mut |o| {
                        requires_ids(requires, o, &slab, out)
                    });
                    black_box(out.len())
                },
                BatchSize::SmallInput,
            )
        });
        group.finish();

    }

    {
    let mut group = c.benchmark_group(format!("{fixture}/storage/serialize"));
    group.throughput(Throughput::Bytes(bytes));
    group.bench_function("slots", |b| {
        b.iter(|| {
            let out = project_by_operation(
                slots_data,
                vec![],
                &Default::default(),
                root_type_name,
                &projection_plan,
                &None,
                size_hint,
                &metadata,
            )
            .expect("projection");
            black_box(out.len())
        })
    });
    group.bench_function("slots_same_effort", |b| {
        b.iter_batched_ref(
            || Vec::<u8>::with_capacity(size_hint),
            |out| {
                project_slots_simple(&projection_plan, slots_data, out);
                black_box(out.len())
            },
            BatchSize::SmallInput,
        )
    });
    if !has_passthrough {
        group.bench_function("stride", |b| {
            b.iter_batched_ref(
                || Vec::<u8>::with_capacity(size_hint),
                |out| {
                    project_stride(&projection_plan, &stride_data, out);
                    black_box(out.len())
                },
                BatchSize::SmallInput,
            )
        });
    }
    group.bench_function("ids", |b| {
        b.iter_batched_ref(
            || Vec::<u8>::with_capacity(size_hint),
            |out| {
                project_ids(&projection_plan, ids_data, &slab, out);
                black_box(out.len())
            },
            BatchSize::SmallInput,
        )
    });
    group.bench_function("positional_no_plan", |b| {
        b.iter_batched_ref(
            || Vec::<u8>::with_capacity(size_hint),
            |out| {
                serialize_positional(pos_data, &pos_slab, out);
                black_box(out.len())
            },
            BatchSize::SmallInput,
        )
    });
    group.finish();
    }
}

criterion_group!(benches, storage_layouts);
criterion_main!(benches);
