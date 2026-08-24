//! Allocation profile of the response pipeline.
//!
//! Not a timing benchmark — it counts allocations, so the question "can this allocate less"
//! has an answer instead of an opinion. Run with `cargo bench --bench alloc_profile`.

use bytes::Bytes;
use graphql_tools::parser::query::Definition;
use bumpalo::Bump;
use std::hint::black_box;
use hive_router_plan_executor::{
    introspection::schema::{SchemaMetadata, SchemaWithMetadata},
    projection::{plan::FieldProjectionPlan, response::project_by_operation},
    response::{merge::deep_merge, subgraph_response::SubgraphResponse, value::Value},
};
use hive_router_query_planner::{
    ast::{document::NormalizedDocument, normalization::create_normalized_document},
    consumer_schema::ConsumerSchema,
    planner::{merged_shape::response_shape_for_operation, response_shape::ResponseShape},
    state::supergraph_state::SupergraphState,
    utils::parsing::{parse_operation, parse_schema},
};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

pub mod payloads;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static FREES: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        BYTES.fetch_add(layout.size(), Relaxed);
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        FREES.fetch_add(1, Relaxed);
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        BYTES.fetch_add(new_size.saturating_sub(layout.size()), Relaxed);
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

struct Counts {
    allocs: usize,
    frees: usize,
    bytes: usize,
}

fn measure<T>(f: impl FnOnce() -> T) -> (T, Counts) {
    let (a0, f0, b0) = (
        ALLOCS.load(Relaxed),
        FREES.load(Relaxed),
        BYTES.load(Relaxed),
    );
    let out = f();
    let counts = Counts {
        allocs: ALLOCS.load(Relaxed) - a0,
        frees: FREES.load(Relaxed) - f0,
        bytes: BYTES.load(Relaxed) - b0,
    };
    (out, counts)
}

const SDL: &str = r#"
    type Query { articles: [Article!]! users: [User!]! }
    type Article { id: ID! title: String! author: String! views: Int! tags: [String] }
    type User { id: ID! name: String! username: String! email: String! bio: String! }
"#;

struct Fixture {
    metadata: &'static SchemaMetadata,
    root_type_name: &'static str,
    plan: Vec<FieldProjectionPlan>,
    shape: ResponseShape,
}

fn fixture(operation_source: &str) -> Fixture {
    let supergraph = parse_schema(SDL);
    let consumer_schema = ConsumerSchema::new_from_supergraph(&supergraph);
    let metadata: &'static SchemaMetadata = Box::leak(Box::new(consumer_schema.schema_metadata()));
    let mut operation = parse_operation(operation_source);
    let operation_ast = operation
        .definitions
        .iter_mut()
        .find_map(|def| match def {
            Definition::Operation(op) => Some(op),
            _ => None,
        })
        .expect("operation");
    let supergraph_state = SupergraphState::new(&supergraph);
    let normalized: &'static NormalizedDocument = Box::leak(Box::new(create_normalized_document(
        &supergraph_state,
        operation_ast.clone(),
        None,
    )));
    let (root_type_name, plan) =
        FieldProjectionPlan::from_operation(&normalized.operation, metadata);
    Fixture {
        metadata,
        root_type_name,
        plan,
        shape: response_shape_for_operation(&normalized.operation),
    }
}

fn report(label: &str, objects: usize, c: &Counts) {
    let per_obj = if objects > 0 {
        c.allocs as f64 / objects as f64
    } else {
        0.0
    };
    println!(
        "{label:<34} allocs={:>7}  frees={:>7}  bytes={:>9}  allocs/object={per_obj:>5.2}",
        c.allocs, c.frees, c.bytes
    );
}

fn main() {
    // 200 articles: 1 root + 1 array + 200 objects = 202 objects' worth of structure.
    let rows = 200;
    let objects = rows + 1;

    let f = fixture("{ articles { id title author views tags } }");
    let raw_shape = {
        let mut s = f.shape.clone();
        s.insert_raw_path(["articles", "tags"]);
        s
    };
    let json = payloads::mixed(rows, 10);
    let size_hint = json.len() * 12 / 10;
    let bytes = Bytes::from(json);

    println!("size_of::<Value>() = {}", std::mem::size_of::<Value>());
    println!(
        "size_of::<ResponseShapeField>() = {}",
        std::mem::size_of::<hive_router_query_planner::planner::response_shape::ResponseShapeField>(
        )
    );
    println!("objects in payload = {objects} (1 root + {rows} articles)\n");

    for (variant, shape) in [("structured", &f.shape), ("raw(tags)", &raw_shape)] {
        let (response, c) = measure(|| {
            SubgraphResponse::deserialize_from_bytes(bytes.clone(), Some(shape)).unwrap()
        });
        report(&format!("deserialize [{variant}]"), objects, &c);

        let (_, c) = measure(|| {
            project_by_operation(
                &response.data,
                vec![],
                &Default::default(),
                f.root_type_name,
                &f.plan,
                &None,
                size_hint,
                f.metadata,
            )
            .unwrap()
        });
        report(&format!("project [{variant}]"), objects, &c);

        // Tearing down a response is dropping its arena and its buffer, not walking the
        // tree: `Value` has no `Drop` glue at all.
        let (_, c) = measure(|| drop(response));
        report(&format!("drop response [{variant}]"), objects, &c);
        println!();
    }

    // Merge: two responses landing in the same position.
    let a = SubgraphResponse::deserialize_from_bytes(bytes.clone(), Some(&f.shape)).unwrap();
    let b = SubgraphResponse::deserialize_from_bytes(bytes.clone(), Some(&f.shape)).unwrap();
    let scratch = Bump::new();
    let target = a.data.copy_into(&scratch);
    let source = b.data.copy_into(&scratch);
    let (merged, c) = measure(|| {
        let mut t = target;
        deep_merge(&mut t, source);
        t
    });
    report("deep_merge (positional)", objects, &c);
    // `merged` borrows `scratch`, so what there is to free is the arena, and only the arena.
    black_box(&merged);
    let (_, c) = measure(|| drop(scratch));
    report("drop merge arena", objects, &c);
}
