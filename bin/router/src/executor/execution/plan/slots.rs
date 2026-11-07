//! Runs a query plan on the slot pipeline (see `executor::slots`). The same plan walk, job
//! preparation and error handling as the `Value` pipeline in the parent module, but subgraph
//! bodies are read into slots, representations are written from slots, and entities are merged
//! by copying slots.

use super::*;
use crate::executor::executors::error::SubgraphExecutorError;
use crate::executor::introspection::schema::PossibleTypes;
use crate::executor::projection::response::serialize_value_to_buffer;
use crate::executor::slots::compile::{
    AliasProgram, FetchProgram, Layout, Requires, RequiresNode, SlotPlan, Step, NO_SLOT,
};
use crate::executor::slots::parse::parse_body;
use crate::executor::slots::store::{Arena, Slot, SlotView};
use crate::executor::slots::write::write_response;
use crate::executor::slots::SlotPlanCell;
use crate::executor::utils::consts::TYPENAME_JSON_FIELD;
use crate::query_planner::planner::plan_nodes::PathSegment;
use smallvec::SmallVec;

/// No entity for a spot: it's null, or it has nothing to send.
const NO_ENTITY: u32 = u32::MAX;

/// Whether this request can run on the slot pipeline. The plan decides most of it when it's
/// compiled; what's left is per router and per request.
pub(super) fn usable(opts: &QueryPlanExecutionOpts<'_>) -> Option<Arc<SlotPlan>> {
    if !crate::executor::slots::enabled()
        || opts.plugin_req_state.is_some()
        || opts.introspection_context.query.is_some()
        || !(opts.operation_kind.is_query() || opts.operation_kind.is_mutation())
    {
        return None;
    }
    opts.slot_plan.as_deref().and_then(|cell: &SlotPlanCell| {
        cell.get(
            opts.query_plan,
            &opts.projection_plan,
            &opts.introspection_context.metadata,
            &opts.operation_type_name,
        )
    })
}

/// Runs the plan on the slot pipeline, with the slot plan [`usable`] gave
pub(super) async fn execute<'exec>(
    mut opts: QueryPlanExecutionOpts<'exec>,
    plan: Arc<SlotPlan>,
) -> Result<PlanExecutionOutput, PlanExecutionError> {
    let plan = &*plan;
    let aliased_requests = AliasedRequests::default();
    let executor = Executor {
        variable_values: &opts.variable_values.variables_map,
        schema_metadata: &opts.introspection_context.metadata,
        executors: &opts.executors,
        client_request: &opts.client_request,
        headers_plan: &opts.headers_plan,
        extensions_plan: &opts.extensions_plan,
        jwt_forwarding_plan: opts.jwt_auth_forwarding.clone(),
        // a mutation's subgraph calls each run, like on the other pipeline
        dedupe_subgraph_requests: opts.operation_kind.is_query(),
        demand_control_context: opts.demand_control_context.clone(),
        plugin_req_state: None,
        operation_name_factory: &opts.operation_name_factory,
        connection_fingerprint: opts.connection_fingerprint,
        aliased_requests: &aliased_requests,
    };
    let mut ctx = ExecutionContext::new(Value::Null, std::mem::take(&mut opts.initial_errors));
    let mut run = Run {
        plan,
        arena: Arena::take(),
        data: Slot::Missing,
    };
    let width = plan.layouts[plan.root_layout as usize].width();
    run.data = Slot::Object {
        layout: plan.root_layout,
        first: run.arena.object(width),
    };

    if let Some(node) = &opts.query_plan.node {
        run.node(&executor, &mut ctx, node).await;
    }

    let error_count = ctx.errors.len();
    if error_count > 0 {
        opts.span.record_error_count(error_count);
        opts.span
            .record_errors(|| ctx.errors.iter().map(|e| e.into()).collect());
        if let Some(error_recorder) = opts.graphql_error_recorder.as_ref() {
            error_recorder
                .record_errors(|| ctx.errors.iter().map(|err| err.extensions.code.as_deref()));
        }
    }
    summary::record(|s| s.set_partial_response(error_count > 0));
    let response_size_estimate = ctx.response_storage.estimate_final_response_size();
    let mut errors = std::mem::take(&mut ctx.errors);

    if let Some(demand_control) = executor.demand_control_context.as_deref() {
        let actual = demand_control.calculate_actual_cost(
            SlotView {
                arena: &run.arena,
                layouts: &plan.layouts,
                slot: run.data,
            },
            &opts.variable_values.variables_map,
            &ctx.subgraph_response_cost_tracker,
        );
        if actual > demand_control.operation.operation_max_cost {
            warn!(
                target: targets::DEMAND_CONTROL,
                operation_name = ?opts.operation_for_plan.name.as_deref(),
                actual_cost = actual,
                estimated_cost = demand_control.evaluation.estimated_cost,
                max_cost = demand_control.operation.operation_max_cost,
                "actual cost exceeds max cost (not enforced)"
            );
        }
        demand_control.report_telemetry(
            actual,
            opts.operation_for_plan.name.as_deref(),
            &opts.span,
        );
        demand_control.apply_expose_headers(&mut ctx.response_headers_aggregator, actual);
    }

    cache_control::finalize(
        &mut ctx.response_headers_aggregator,
        // force no-store for mutations and errors, like the other pipeline
        opts.operation_kind.is_mutation() || !errors.is_empty(),
        ctx.response_storage.len(),
    );
    opts.response_header_sink
        .store(std::mem::take(&mut ctx.response_headers_aggregator));
    std::mem::take(&mut ctx.extensions_aggregator).merge_into(&mut opts.extensions.extensions);

    if let Some(error_masking_runtime) = opts.error_masking_runtime.as_ref() {
        for error in &mut errors {
            error_masking_runtime.apply(error);
        }
    }

    let body = write_response(
        &run.arena,
        plan,
        run.data,
        errors,
        &opts.extensions,
        opts.operation_type_name.as_str(),
        &opts.variable_values.variables_map,
        response_size_estimate,
        &opts.introspection_context.metadata,
    )
    .with_plan_context(LazyPlanContext {
        subgraph_name: || None,
        affected_path: || None,
    })?;
    run.arena.give_back();

    Ok(PlanExecutionOutput {
        body,
        error_count,
        status_code: StatusCode::OK,
    })
}

struct Run<'p> {
    plan: &'p SlotPlan,
    arena: Arena,
    /// The root object
    data: Slot,
}

/// What a job's response gets merged with. One per job in flight, so the size difference
/// between the variants doesn't matter.
#[allow(clippy::large_enum_variant)]
enum Merge<'exec, 'p> {
    /// Into the root, or the objects at a root re-entry's path
    Root {
        program: &'p FetchProgram,
        merge_path: Option<&'p [Step]>,
    },
    Entities {
        program: &'p FetchProgram,
        aliases: SmallVec<[AliasMerge<'exec, 'p>; 2]>,
        /// Errors are split by alias, like a `BatchFetch`'s
        batch: bool,
    },
}

struct AliasMerge<'exec, 'p> {
    program: &'p AliasProgram,
    /// The alias in the response, for errors
    name: &'exec str,
    paths: SmallVec<[ResponsePathRef<'exec>; 4]>,
    targets: Targets,
}

/// The spots of each path of an alias, in walk order, with the entity each one gets. One
/// `Vec` for all paths, from the arena's pool.
#[derive(Default)]
struct Targets {
    spots: Vec<(Slot, u32)>,
    /// Where each path's spots end
    ends: SmallVec<[u32; 4]>,
}

impl Targets {
    fn paths(&self) -> impl Iterator<Item = &[(Slot, u32)]> {
        let mut start = 0;
        self.ends.iter().map(move |&end| {
            let path = &self.spots[start..end as usize];
            start = end as usize;
            path
        })
    }
}

type Job<'wave, 'exec> = BoxFuture<'wave, (usize, Result<ExecutionJob<'exec>, PlanExecutionError>)>;

impl<'p> Run<'p> {
    fn node<'a, 'exec>(
        &'a mut self,
        executor: &'a Executor<'exec>,
        ctx: &'a mut ExecutionContext<'exec>,
        node: &'exec PlanNode,
    ) -> BoxFuture<'a, ()>
    where
        'exec: 'a,
        'p: 'a,
    {
        async move {
            match node {
                PlanNode::Sequence(sequence) => {
                    for child in &sequence.nodes {
                        self.node(executor, ctx, child).await;
                    }
                }
                PlanNode::Parallel(parallel) => {
                    self.wave(executor, ctx, parallel.nodes.iter()).await;
                }
                node => self.wave(executor, ctx, std::iter::once(node)).await,
            }
        }
        .boxed()
    }

    /// Sends the jobs of `nodes` together and merges each response as it arrives.
    async fn wave<'exec>(
        &mut self,
        executor: &Executor<'exec>,
        ctx: &mut ExecutionContext<'exec>,
        nodes: impl Iterator<Item = &'exec PlanNode>,
    ) {
        let mut jobs: FuturesUnordered<Job<'_, 'exec>> = FuturesUnordered::new();
        let mut merges: SmallVec<[Option<Merge<'exec, 'p>>; 4]> = SmallVec::new();
        for node in nodes {
            // nothing is copied once the request has an error: a null from an error isn't data
            let reuse = ctx.errors.is_empty();
            if let Some((job, merge)) = self.prepare(executor, node, merges.len(), reuse) {
                jobs.push(job);
                merges.push(Some(merge));
            }
        }
        while let Some((index, job)) = jobs.next().await {
            let merge = merges[index].take().expect("each job finishes once");
            match job {
                Err(ref err) => executor.handle_job_error(ctx, err),
                Ok(job) => self.process(executor, ctx, job, merge),
            }
        }
    }

    fn prepare<'wave, 'exec>(
        &mut self,
        executor: &'wave Executor<'exec>,
        node: &'exec PlanNode,
        index: usize,
        reuse: bool,
    ) -> Option<(Job<'wave, 'exec>, Merge<'exec, 'p>)> {
        let plan = self.plan;
        let send = |fetch: &'exec FetchNode,
                    variables: Option<Vec<(&'exec str, Vec<u8>)>>,
                    affected_path: Option<ResponsePathRef<'exec>>|
         -> Job<'wave, 'exec> {
            executor
                .prepare_execution_job(PrepareExecutionJobOpts {
                    subgraph_name: &fetch.service_name,
                    variable_usages: fetch.variable_usages.as_ref(),
                    operation_name: executor
                        .operation_name_factory
                        .generate(&fetch.service_name, fetch.id),
                    operation_kind: fetch.operation_kind.as_ref(),
                    operation: &fetch.operation,
                    output_rewrites: None,
                    custom_scalar_paths: fetch.custom_scalar_paths.as_ref(),
                    raw_variable_values: variables,
                    affected_path,
                    raw_response: true,
                })
                .map(move |job| (index, job))
                .boxed()
        };
        match node {
            PlanNode::Fetch(fetch) => {
                if !executor.runs(fetch.run_if.as_deref()) {
                    return None;
                }
                let program = plan.fetches.get(&fetch.id)?;
                Some((
                    send(fetch, None, None),
                    Merge::Root {
                        program,
                        merge_path: None,
                    },
                ))
            }
            PlanNode::Flatten(flatten) => {
                let PlanNode::Fetch(fetch) = flatten.node.as_ref() else {
                    return None;
                };
                if !executor.runs(fetch.run_if.as_deref()) {
                    return None;
                }
                let program = plan.fetches.get(&fetch.id)?;
                let path = flatten.path.as_ref();
                if fetch.requires.is_none() {
                    return Some((
                        send(fetch, None, Some(path)),
                        Merge::Root {
                            program,
                            merge_path: program.merge_path.as_deref(),
                        },
                    ));
                }
                let alias = program.aliases.first()?;
                let keys = executor
                    .schema_metadata
                    .entity_keys
                    .of_subgraph(&fetch.service_name);
                let (representations, count, targets) =
                    self.collect(alias, &executor.schema_metadata.possible_types, keys, reuse);
                if count == 0 {
                    return None;
                }
                Some((
                    send(
                        fetch,
                        Some(vec![("representations", representations)]),
                        Some(path),
                    ),
                    Merge::Entities {
                        program,
                        aliases: smallvec::smallvec![AliasMerge {
                            program: alias,
                            name: "_entities",
                            paths: smallvec::smallvec![path],
                            targets,
                        }],
                        batch: false,
                    },
                ))
            }
            PlanNode::BatchFetch(batch) => {
                let program = plan.fetches.get(&batch.id)?;
                let keys = executor
                    .schema_metadata
                    .entity_keys
                    .of_subgraph(&batch.service_name);
                let mut variables: Vec<(&'exec str, Vec<u8>)> =
                    Vec::with_capacity(batch.entity_batch.aliases.len());
                let mut aliases = SmallVec::new();
                let mut any = false;
                for (spec, alias) in batch.entity_batch.aliases.iter().zip(&program.aliases) {
                    // an alias that's off sends `[]` and has nowhere to merge
                    let (representations, count, targets) = if executor.runs(spec.run_if.as_deref())
                    {
                        self.collect(alias, &executor.schema_metadata.possible_types, keys, reuse)
                    } else {
                        (b"[]".to_vec(), 0, Targets::default())
                    };
                    any |= count > 0;
                    let name = spec.representations_variable_name.as_str();
                    if !variables.iter().any(|(existing, _)| *existing == name) {
                        variables.push((name, representations));
                    }
                    aliases.push(AliasMerge {
                        program: alias,
                        name: spec.alias.as_str(),
                        paths: if targets.ends.is_empty() {
                            SmallVec::new()
                        } else {
                            spec.merge_paths.iter().collect()
                        },
                        targets,
                    });
                }
                if !any {
                    return None;
                }
                let job = executor
                    .prepare_execution_job(PrepareExecutionJobOpts {
                        subgraph_name: &batch.service_name,
                        variable_usages: batch.variable_usages.as_ref(),
                        operation_name: executor
                            .operation_name_factory
                            .generate(&batch.service_name, batch.id),
                        operation_kind: batch.operation_kind.as_ref(),
                        operation: &batch.operation,
                        output_rewrites: None,
                        custom_scalar_paths: batch.custom_scalar_paths.as_ref(),
                        raw_variable_values: Some(variables),
                        affected_path: None,
                        raw_response: true,
                    })
                    .map(move |job| (index, job))
                    .boxed();
                Some((
                    job,
                    Merge::Entities {
                        program,
                        aliases,
                        batch: true,
                    },
                ))
            }
            _ => None,
        }
    }

    /// Walks the alias's paths and writes a representation for every distinct entity. Returns
    /// the representations as a JSON list, how many there are, and the entity of every spot.
    /// With `reuse`, a spot that matches an entity already in the response gets its fields
    /// copied from it instead, and no entity.
    fn collect(
        &mut self,
        alias: &AliasProgram,
        possible_types: &PossibleTypes,
        keys: Option<&KeysByType>,
        reuse: bool,
    ) -> (Vec<u8>, u32, Targets) {
        let layouts = &self.plan.layouts;
        let arena = &mut self.arena;
        let mut spots = std::mem::take(&mut arena.spots);
        let mut frontier = std::mem::take(&mut arena.frontier);
        let mut seen = std::mem::take(&mut arena.seen);
        let mut ranges = std::mem::take(&mut arena.ranges);
        let mut prints = std::mem::take(&mut arena.prints);
        seen.clear();
        ranges.clear();
        prints.clear();
        let mut reps = Vec::with_capacity(256);
        reps.push(b'[');
        let mut count = 0u32;
        let hasher = ahash::RandomState::with_seeds(7, 11, 13, 17);
        let mut targets = Targets {
            spots: arena.targets.pop().unwrap_or_default(),
            ends: SmallVec::new(),
        };
        let mut known = std::mem::take(&mut arena.known);
        known.clear();
        let mut key_slot_cache = SmallVec::new();
        let reuse = alias.reuse.as_ref().filter(|_| reuse);
        if let Some(reuse) = reuse {
            // the entities already in the response, by what their representation would read
            for (index, source) in reuse.sources.iter().enumerate() {
                walk(
                    arena,
                    layouts,
                    possible_types,
                    self.data,
                    &source.path,
                    &mut spots,
                    &mut frontier,
                );
                for &object in &spots {
                    let Slot::Object { first, .. } = object else {
                        continue;
                    };
                    if !holds_representation(arena, &source.requires, first)
                        || source.copy.iter().any(|&slot| {
                            arena.slots[(first + slot as u32) as usize] == Slot::Missing
                        })
                    {
                        continue;
                    }
                    let mut print = Fingerprint {
                        arena,
                        layouts,
                        requires: &source.requires,
                        hasher: Mix::new(),
                    };
                    print.value(object, source.requires.reads_root.clone(), None);
                    known
                        .entry(print.hasher.finish())
                        .or_insert((first, index as u32));
                }
            }
        }
        for path in &alias.paths {
            spots.clear();
            walk(
                arena,
                layouts,
                possible_types,
                self.data,
                path,
                &mut spots,
                &mut frontier,
            );
            let path_targets = &mut targets.spots;
            for &spot in &spots {
                if let (Some(reuse), Slot::Object { layout, first }) = (reuse, spot) {
                    if layout == reuse.target_layout && !known.is_empty() {
                        // without the key mask: it depends on the layout, and the source's
                        // isn't the spot's
                        let mut print = Fingerprint {
                            arena,
                            layouts,
                            requires: &alias.requires,
                            hasher: Mix::new(),
                        };
                        print.value(spot, alias.requires.reads_root.clone(), None);
                        if let Some(&(from, source)) = known.get(&print.hasher.finish()) {
                            let from_slots = &reuse.sources[source as usize].copy;
                            for (&to, &from_slot) in reuse.copy.iter().zip(from_slots.iter()) {
                                arena.slots[(first + to as u32) as usize] =
                                    arena.slots[(from + from_slot as u32) as usize];
                            }
                            path_targets.push((spot, NO_ENTITY));
                            continue;
                        }
                    }
                }
                let entity = match spot {
                    Slot::Null | Slot::Missing => NO_ENTITY,
                    _ => {
                        // most spots repeat an entity another spot already sent: tell them
                        // apart by what the representation would read, without writing it
                        let mut print = Fingerprint {
                            arena,
                            layouts,
                            requires: &alias.requires,
                            hasher: Mix::new(),
                        };
                        let key_slots = match (keys, spot) {
                            (Some(keys), Slot::Object { layout, .. }) => {
                                Some(key_slots(&mut key_slot_cache, layouts, keys, layout))
                            }
                            _ => None,
                        };
                        print.value(spot, alias.requires.reads_root.clone(), key_slots);
                        let fingerprint = print.hasher.finish();
                        if let Some(&entity) = prints.get(&fingerprint) {
                            path_targets.push((spot, entity));
                            continue;
                        }
                        let before = reps.len();
                        let first_rep = count == 0;
                        let written = Representation {
                            arena,
                            layouts,
                            requires: &alias.requires,
                            possible_types,
                            buffer: &mut reps,
                        }
                        .value(
                            spot,
                            alias.requires.root.clone(),
                            first_rep,
                            None,
                            false,
                            keys,
                        );
                        let entity = if !written {
                            reps.truncate(before);
                            NO_ENTITY
                        } else {
                            let start = if first_rep { before } else { before + 1 };
                            let hash = hasher.hash_one(&reps[start..]);
                            match seen.get(&hash).copied().filter(|&index| {
                                let (s, l) = ranges[index as usize];
                                reps[s as usize..(s + l) as usize] == reps[start..]
                            }) {
                                Some(index) => {
                                    reps.truncate(before);
                                    index
                                }
                                None => {
                                    seen.insert(hash, count);
                                    ranges.push((start as u32, (reps.len() - start) as u32));
                                    count += 1;
                                    count - 1
                                }
                            }
                        };
                        prints.insert(fingerprint, entity);
                        entity
                    }
                };
                path_targets.push((spot, entity));
            }
            targets.ends.push(targets.spots.len() as u32);
        }
        reps.push(b']');
        arena.spots = spots;
        arena.frontier = frontier;
        arena.seen = seen;
        arena.ranges = ranges;
        arena.prints = prints;
        arena.known = known;
        (reps, count, targets)
    }

    fn process<'exec>(
        &mut self,
        executor: &Executor<'exec>,
        ctx: &mut ExecutionContext<'exec>,
        job: ExecutionJob<'exec>,
        merge: Merge<'exec, 'p>,
    ) {
        let subgraph_name = job.subgraph_name();
        let affected_path = job.affected_path();
        let fetch_hash = job.operation().hash;
        let response = job.response();
        if let Some(ref headers) = response.headers {
            executor.apply_job_headers(ctx, subgraph_name, affected_path, headers);
        }
        let program = match &merge {
            Merge::Root { program, .. } | Merge::Entities { program, .. } => *program,
        };

        // an executor that can't hand over the body parsed it: write `data` back out
        let (bytes, mut errors) = match response.bytes {
            Some(bytes) => (bytes, None),
            None => {
                let mut body = b"{\"data\":".to_vec();
                serialize_value_to_buffer(&response.data, &mut body);
                body.push(b'}');
                (bytes::Bytes::from(body), response.errors)
            }
        };
        let status_errors = response
            .status
            .filter(|status| !status.is_success())
            .map(|status| GraphQLError::create_from_status_code(&status, subgraph_name));
        let layouts = &self.plan.layouts;
        // demand control counts the lists of a response, which a passed-through subtree hides
        let passthrough = executor.demand_control_context.is_none();
        let parsed = parse_body(
            &mut self.arena,
            layouts,
            &program.shape,
            &bytes,
            passthrough,
        );
        let Some(parsed) = parsed else {
            // sonic says what's wrong with it, like it does for the other pipeline
            let error = match SubgraphResponse::deserialize_from_bytes(bytes, None) {
                Err(SubgraphExecutorError::ResponseDeserializationFailure(err, _)) => {
                    SubgraphExecutorError::ResponseDeserializationFailure(
                        err,
                        response.headers.clone(),
                    )
                }
                Err(other) => other,
                Ok(_) => SubgraphExecutorError::MalformedResponse(response.headers.clone()),
            };
            let error = Err::<(), _>(error)
                .with_plan_context(LazyPlanContext {
                    subgraph_name: || Some(subgraph_name.to_string()),
                    affected_path: || affected_path.map(|p| p.to_string()),
                })
                .expect_err("an error");
            executor.handle_job_error(ctx, &error);
            return;
        };
        ctx.response_storage.add_response(bytes.clone());
        if let Some(demand_control) = executor.demand_control_context.as_deref() {
            demand_control.record_subgraph_data_cost(
                &mut ctx.subgraph_response_cost_tracker,
                subgraph_name,
                fetch_hash,
                SlotView {
                    arena: &self.arena,
                    layouts,
                    slot: parsed.data,
                },
                executor.variable_values,
            );
        }
        if matches!(parsed.data, Slot::Missing | Slot::Null)
            && parsed.errors.is_none()
            && errors.is_none()
        {
            let error = Err::<(), _>(SubgraphExecutorError::MalformedResponse(None))
                .with_plan_context(LazyPlanContext {
                    subgraph_name: || Some(subgraph_name.to_string()),
                    affected_path: || affected_path.map(|p| p.to_string()),
                })
                .expect_err("an error");
            executor.handle_job_error(ctx, &error);
            return;
        }
        if errors.is_none() {
            errors = parsed.errors;
        }
        if let Some(status_error) = status_errors {
            errors.get_or_insert_with(Vec::new).push(status_error);
        }
        if parsed.extensions {
            if let Some(extensions) = SubgraphResponse::deserialize_from_bytes(bytes, None)
                .ok()
                .and_then(|response| response.extensions)
            {
                apply_subgraph_extensions(
                    executor.extensions_plan,
                    &extensions,
                    &mut ctx.extensions_aggregator,
                );
            }
        }

        match merge {
            Merge::Root { merge_path, .. } => {
                let source = parsed.data;
                match merge_path {
                    None => {
                        if let (Slot::Object { layout, first }, Slot::Object { first: root, .. }) =
                            (source, self.data)
                        {
                            self.arena.merge_object(layouts, layout, root, first);
                        }
                    }
                    Some(steps) => {
                        let mut spots = std::mem::take(&mut self.arena.spots);
                        let mut frontier = std::mem::take(&mut self.arena.frontier);
                        walk(
                            &self.arena,
                            layouts,
                            &executor.schema_metadata.possible_types,
                            self.data,
                            steps,
                            &mut spots,
                            &mut frontier,
                        );
                        self.arena.frontier = frontier;
                        if let Slot::Object { layout, first } = source {
                            for spot in &spots {
                                if let Slot::Object {
                                    layout: target_layout,
                                    first: target,
                                } = *spot
                                {
                                    if target_layout == layout {
                                        self.arena.merge_object(layouts, layout, target, first);
                                    }
                                }
                            }
                        }
                        self.arena.spots = spots;
                    }
                }
                ctx.handle_errors(subgraph_name, affected_path, errors, None);
            }
            Merge::Entities { aliases, batch, .. } => {
                let mut by_alias: SmallVec<[Option<Vec<GraphQLError>>; 4]> =
                    smallvec::smallvec![None; aliases.len()];
                let mut unmatched = Vec::new();
                if batch {
                    for mut error in errors.into_iter().flatten() {
                        let alias =
                            error
                                .path
                                .as_ref()
                                .and_then(|path| match path.segments.first() {
                                    Some(GraphQLErrorPathSegment::String(name)) => {
                                        aliases.iter().position(|alias| alias.name == name)
                                    }
                                    _ => None,
                                });
                        match alias {
                            Some(index) => {
                                if let Some(GraphQLErrorPathSegment::String(first)) = error
                                    .path
                                    .as_mut()
                                    .and_then(|path| path.segments.first_mut())
                                {
                                    *first = "_entities".to_string();
                                }
                                by_alias[index].get_or_insert_with(Vec::new).push(error);
                            }
                            None => unmatched.push(error),
                        }
                    }
                } else if let Some(first) = by_alias.first_mut() {
                    *first = errors;
                }

                for (alias, alias_errors) in aliases.iter().zip(by_alias) {
                    let entities = match parsed.data {
                        Slot::Object { first, .. } if alias.program.slot != NO_SLOT => {
                            match self.arena.slots[(first + alias.program.slot as u32) as usize] {
                                Slot::List { start, len } => Some((start, len)),
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    // where each entity went, for errors that point into `_entities`
                    let mut error_map: Option<HashMap<usize, Vec<GraphQLErrorPath>>> =
                        (batch && alias_errors.is_some()).then(HashMap::new);
                    if let Some((start, len)) = entities {
                        if alias_errors.is_some() {
                            let map = error_map.get_or_insert_with(HashMap::new);
                            for (path, targets) in alias.paths.iter().zip(alias.targets.paths()) {
                                let mut error_paths = Vec::with_capacity(targets.len());
                                error_walk(
                                    &self.arena,
                                    layouts,
                                    &executor.schema_metadata.possible_types,
                                    self.data,
                                    path.as_slice(),
                                    &mut GraphQLErrorPath::with_capacity(path.len() + 2),
                                    &mut error_paths,
                                );
                                for ((_, entity), error_path) in targets.iter().zip(error_paths) {
                                    if *entity != NO_ENTITY {
                                        map.entry(*entity as usize).or_default().push(error_path);
                                    }
                                }
                            }
                        }
                        {
                            for &(spot, entity) in &alias.targets.spots {
                                if entity == NO_ENTITY || entity >= len {
                                    continue;
                                }
                                let item = self.arena.items[(start + entity) as usize];
                                if let (
                                    Slot::Object { layout, first },
                                    Slot::Object {
                                        layout: target_layout,
                                        first: target,
                                    },
                                ) = (item, spot)
                                {
                                    if layout == target_layout && first != target {
                                        self.arena.merge_object(layouts, layout, target, first);
                                    }
                                }
                            }
                        }
                    }
                    let alias_path = if batch {
                        (alias.paths.len() == 1).then(|| alias.paths[0])
                    } else {
                        affected_path
                    };
                    ctx.handle_errors(subgraph_name, alias_path, alias_errors, error_map);
                }
                if !unmatched.is_empty() {
                    ctx.handle_errors(subgraph_name, None, Some(unmatched), None);
                }
                for alias in aliases {
                    self.arena.give_back_targets(alias.targets.spots);
                }
            }
        }
    }
}

/// Collects the values at the end of `steps`, like `traverse_and_callback`: a list at the end
/// gives its items, a missing field or a value of the wrong kind gives nothing. It goes one
/// step at a time over all values at that depth (`frontier` is scratch space); expanding each
/// level in order keeps the document order a depth-first walk has.
fn walk(
    arena: &Arena,
    layouts: &[Layout],
    possible_types: &PossibleTypes,
    root: Slot,
    steps: &[Step],
    out: &mut Vec<Slot>,
    frontier: &mut Vec<Slot>,
) {
    out.clear();
    out.push(root);
    for step in steps {
        std::mem::swap(out, frontier);
        out.clear();
        for &value in frontier.iter() {
            match step {
                Step::Field(slot) => {
                    if let Slot::Object { first, .. } = value {
                        if *slot != NO_SLOT {
                            let child = arena.slots[(first + *slot as u32) as usize];
                            if child != Slot::Missing {
                                out.push(child);
                            }
                        }
                    }
                }
                Step::List => {
                    if let Slot::List { start, len } = value {
                        out.extend_from_slice(arena.list_items(start, len));
                    }
                }
                Step::TypeCondition(condition) => type_condition(
                    arena,
                    layouts,
                    possible_types,
                    value,
                    condition.names(),
                    out,
                ),
            }
        }
    }
    // a list at the end gives its items
    if out.iter().any(|value| matches!(value, Slot::List { .. })) {
        std::mem::swap(out, frontier);
        out.clear();
        for &value in frontier.iter() {
            match value {
                Slot::List { start, len } => out.extend_from_slice(arena.list_items(start, len)),
                value => out.push(value),
            }
        }
    }
}

/// An object passes when it meets the condition; a list is checked item by item.
fn type_condition(
    arena: &Arena,
    layouts: &[Layout],
    possible_types: &PossibleTypes,
    value: Slot,
    names: &[Box<str>],
    out: &mut Vec<Slot>,
) {
    match value {
        Slot::Object { layout, first } => {
            if satisfies(arena, layouts, possible_types, layout, first, names) {
                out.push(value);
            }
        }
        Slot::List { start, len } => {
            for i in 0..len {
                let item = arena.items[(start + i) as usize];
                type_condition(arena, layouts, possible_types, item, names, out);
            }
        }
        _ => {}
    }
}

/// The error path of every spot `walk` finds, in the same order. Only run for a response
/// with errors.
fn error_walk(
    arena: &Arena,
    layouts: &[Layout],
    possible_types: &PossibleTypes,
    value: Slot,
    path: &[PathSegment],
    error_path: &mut GraphQLErrorPath,
    out: &mut Vec<GraphQLErrorPath>,
) {
    let Some((step, rest)) = path.split_first() else {
        match value {
            Slot::List { len, .. } => {
                for i in 0..len {
                    error_path.push_index(i as usize);
                    out.push(error_path.clone());
                    error_path.pop();
                }
            }
            _ => out.push(error_path.clone()),
        }
        return;
    };
    match step {
        PathSegment::Field(key) => {
            if let Slot::Object { layout, first } = value {
                let slot = layouts[layout as usize].slot(key);
                if slot != NO_SLOT {
                    let child = arena.slots[(first + slot as u32) as usize];
                    if child != Slot::Missing {
                        error_path.push_field(key);
                        error_walk(arena, layouts, possible_types, child, rest, error_path, out);
                        error_path.pop();
                    }
                }
            }
        }
        PathSegment::List => {
            if let Slot::List { start, len } = value {
                for i in 0..len {
                    error_path.push_index(i as usize);
                    let item = arena.items[(start + i) as usize];
                    error_walk(arena, layouts, possible_types, item, rest, error_path, out);
                    error_path.pop();
                }
            }
        }
        PathSegment::TypeCondition(condition) => match value {
            Slot::Object { layout, first } => {
                if satisfies(
                    arena,
                    layouts,
                    possible_types,
                    layout,
                    first,
                    condition.names(),
                ) {
                    error_walk(arena, layouts, possible_types, value, rest, error_path, out);
                }
            }
            Slot::List { start, len } => {
                for i in 0..len {
                    error_path.push_index(i as usize);
                    let item = arena.items[(start + i) as usize];
                    error_walk(arena, layouts, possible_types, item, path, error_path, out);
                    error_path.pop();
                }
            }
            _ => {}
        },
    }
}

/// Whether an object's `__typename` meets one of `names`. One without `__typename` does.
fn satisfies(
    arena: &Arena,
    layouts: &[Layout],
    possible_types: &PossibleTypes,
    layout: u32,
    first: u32,
    names: &[Box<str>],
) -> bool {
    let slot = layouts[layout as usize].typename;
    let typename = (slot != NO_SLOT)
        .then(|| arena.string(arena.slots[(first + slot as u32) as usize]))
        .flatten();
    typename.is_none_or(|typename| {
        names
            .iter()
            .any(|name| possible_types.entity_satisfies_type_condition(&typename, name))
    })
}

/// What the selections that apply to an object asked for, see `projection::request`.
#[derive(Default)]
struct Applied {
    typename: bool,
    missing_field: bool,
}

/// Writes representations from slots. A port of `projection::request::project_requires`: the
/// same rules, read from slots.
struct Representation<'r> {
    arena: &'r Arena,
    layouts: &'r [Layout],
    requires: &'r Requires,
    possible_types: &'r PossibleTypes,
    buffer: &'r mut Vec<u8>,
}

impl<'r> Representation<'r> {
    fn key(&mut self, first: bool, response_key: Option<&str>) {
        if !first {
            self.buffer.push(b',');
        }
        if let Some(key) = response_key {
            self.buffer.push(b'"');
            self.buffer.extend_from_slice(key.as_bytes());
            self.buffer.extend_from_slice(b"\":");
        }
    }

    fn typename_field(&mut self, type_name: &str) {
        self.buffer.extend_from_slice(TYPENAME_JSON_FIELD);
        crate::executor::json_writer::write_and_escape_string(self.buffer, type_name);
    }

    fn typename_of(&self, layout: u32, first: u32) -> Option<&'r str> {
        let arena: &'r Arena = self.arena;
        let slot = self.layouts[layout as usize].typename;
        if slot == NO_SLOT {
            return None;
        }
        arena.plain_str(arena.slots[(first + slot as u32) as usize])
    }

    #[allow(clippy::too_many_arguments)]
    fn value(
        &mut self,
        value: Slot,
        selections: std::ops::Range<u32>,
        first: bool,
        response_key: Option<&str>,
        nested: bool,
        keys: Option<&KeysByType>,
    ) -> bool {
        match value {
            Slot::Missing | Slot::Null => return false,
            Slot::True | Slot::False | Slot::Raw(_) => {
                self.key(first, response_key);
                self.arena.write_whole(self.layouts, value, self.buffer);
            }
            Slot::List { start, len } => {
                self.key(first, response_key);
                self.buffer.push(b'[');
                let mut first = true;
                for i in 0..len {
                    let item = self.arena.items[(start + i) as usize];
                    if self.value(item, selections.clone(), first, None, nested, keys) {
                        first = false;
                    } else if nested {
                        if !first {
                            self.buffer.push(b',');
                        }
                        self.buffer.extend_from_slice(b"null");
                        first = false;
                    }
                }
                self.buffer.push(b']');
            }
            Slot::Object {
                layout,
                first: object,
            } => {
                if selections.is_empty() {
                    self.key(first, response_key);
                    self.arena.write_whole(self.layouts, value, self.buffer);
                    return true;
                }
                let width = self.layouts[layout as usize].width() as u32;
                if (object..object + width).all(|i| self.arena.slots[i as usize] == Slot::Missing) {
                    return false;
                }
                let parent_first = first;
                let mut first = true;
                let type_name = self.typename_of(layout, object);
                let type_keys =
                    keys.and_then(|keys| type_name.and_then(|type_name| keys.get(type_name)));
                let start = self.buffer.len();
                let applied = self.map(
                    selections,
                    object,
                    type_name,
                    &mut first,
                    response_key,
                    parent_first,
                    type_keys.is_some(),
                );
                if let Some(type_keys) = type_keys {
                    let keys = &self.layouts[layout as usize];
                    let complete = type_keys.iter().any(|fields| {
                        fields.iter().all(|field| {
                            let slot = keys.slot(field);
                            slot != NO_SLOT
                                && self.arena.slots[(object + slot as u32) as usize]
                                    != Slot::Missing
                        })
                    });
                    if !complete {
                        self.buffer.truncate(start);
                        return false;
                    }
                }
                if first && !applied.missing_field && (nested || applied.typename) {
                    if let Some(type_name) = type_name {
                        self.key(parent_first, response_key);
                        self.buffer.push(b'{');
                        self.typename_field(type_name);
                        first = false;
                    }
                }
                if first {
                    return false;
                }
                self.buffer.push(b'}');
            }
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn map(
        &mut self,
        selections: std::ops::Range<u32>,
        object: u32,
        type_name: Option<&'r str>,
        first: &mut bool,
        parent_key: Option<&str>,
        parent_first: bool,
        keys_known: bool,
    ) -> Applied {
        let mut applied = Applied::default();
        let requires = self.requires;
        for node in &requires.nodes[selections.start as usize..selections.end as usize] {
            match node {
                RequiresNode::Field {
                    response_key,
                    by_name,
                    by_alias,
                    children,
                    ..
                } => {
                    if &**response_key == "__typename" {
                        applied.typename = true;
                        continue;
                    }
                    let read = |slot: u16| {
                        (slot != NO_SLOT)
                            .then(|| self.arena.slots[(object + slot as u32) as usize])
                            .filter(|value| *value != Slot::Missing)
                    };
                    let original = match read(*by_name).or_else(|| read(*by_alias)) {
                        Some(value) => value,
                        None if keys_known => Slot::Null,
                        None => {
                            applied.missing_field = true;
                            continue;
                        }
                    };
                    let mut object_start = None;
                    if *first {
                        object_start = Some(self.buffer.len());
                        self.key(parent_first, parent_key);
                        self.buffer.push(b'{');
                        if let Some(type_name) = type_name {
                            self.typename_field(type_name);
                            *first = false;
                        }
                    }
                    if original == Slot::Null {
                        self.key(*first, Some(response_key));
                        self.buffer.extend_from_slice(b"null");
                        *first = false;
                        continue;
                    }
                    if self.value(
                        original,
                        children.clone(),
                        *first,
                        Some(response_key),
                        true,
                        None,
                    ) {
                        *first = false;
                    } else if *first {
                        if let Some(offset) = object_start {
                            self.buffer.truncate(offset);
                        }
                    }
                }
                RequiresNode::InlineFragment {
                    type_condition,
                    children,
                } => {
                    let name = type_name.unwrap_or(type_condition);
                    if self
                        .possible_types
                        .entity_satisfies_type_condition(name, type_condition)
                        || self
                            .possible_types
                            .entity_satisfies_type_condition(type_condition, name)
                    {
                        let fragment = self.map(
                            children.clone(),
                            object,
                            type_name,
                            first,
                            parent_key,
                            parent_first,
                            keys_known,
                        );
                        applied.typename |= fragment.typename;
                        applied.missing_field |= fragment.missing_field;
                    }
                }
            }
        }
        applied
    }
}

/// Whether every field a representation reads at the top is in the object and not null. One
/// with a missing or null field can't be told apart from another one like it, so nothing is
/// copied from it.
fn holds_representation(arena: &Arena, requires: &Requires, first: u32) -> bool {
    requires.reads[requires.reads_root.start as usize..requires.reads_root.end as usize]
        .iter()
        .all(|read| {
            let get = |slot: u16| {
                (slot != NO_SLOT)
                    .then(|| arena.slots[(first + slot as u32) as usize])
                    .filter(|value| *value != Slot::Missing)
            };
            matches!(get(read.by_name).or_else(|| get(read.by_alias)), Some(value) if value != Slot::Null)
        })
}

/// The slots in `layout` of every key field of every type in `keys`, sorted. A spot's
/// complete-key check reads the presence of some of these, depending on its type.
fn key_slots<'c>(
    cache: &'c mut SmallVec<[(u32, Vec<u16>); 2]>,
    layouts: &[Layout],
    keys: &KeysByType,
    layout: u32,
) -> &'c [u16] {
    let index = match cache.iter().position(|(cached, _)| *cached == layout) {
        Some(index) => index,
        None => {
            let layout_keys = &layouts[layout as usize];
            let mut slots: Vec<u16> = keys
                .values()
                .flatten()
                .flatten()
                .map(|field| layout_keys.slot(field))
                // a key field the layout has no slot for is never there
                .filter(|slot| *slot != NO_SLOT)
                .collect();
            slots.sort_unstable();
            slots.dedup();
            cache.push((layout, slots));
            cache.len() - 1
        }
    };
    &cache[index].1
}

/// Hashes everything [`Representation`] reads to write a spot: its `__typename`, which key
/// fields it has, and the values of the selected fields, recursively. The writer is a function
/// of these, so two spots with the same fingerprint get the same representation (64-bit hash,
/// the same trust `Value::to_hash` gets today).
struct Fingerprint<'r> {
    arena: &'r Arena,
    layouts: &'r [Layout],
    requires: &'r Requires,
    hasher: Mix,
}

impl<'r> Fingerprint<'r> {
    /// `reads` are the fields to hash when `value` is an object, see `Requires::reads`
    /// `key_slots`: the spot's slots of every key field of the subgraph, see [`key_slots`]
    fn value(&mut self, value: Slot, reads: std::ops::Range<u32>, key_slots: Option<&[u16]>) {
        match value {
            Slot::Missing => self.hasher.word(0),
            Slot::Null => self.hasher.word(1),
            Slot::True => self.hasher.word(2),
            Slot::False => self.hasher.word(3),
            Slot::Raw(raw) => self.hasher.bytes(4, self.arena.raw(raw)),
            Slot::List { start, len } => {
                self.hasher.word(5 | (len as u64) << 8);
                for i in 0..len {
                    let item = self.arena.items[(start + i) as usize];
                    self.value(item, reads.clone(), key_slots);
                }
            }
            Slot::Object { layout, first } => {
                let width = self.layouts[layout as usize].width() as u32;
                if reads.is_empty() {
                    // written whole: every slot counts
                    self.hasher.word(6);
                    for i in first..first + width {
                        self.value(self.arena.slots[i as usize], 0..0, None);
                    }
                    return;
                }
                // the typename as written: equal bytes mean an equal name, so the same
                // fragments apply; different bytes only make the fingerprint differ, which is
                // always safe
                let typename = self.layouts[layout as usize].typename;
                match (typename != NO_SLOT)
                    .then(|| self.arena.slots[(first + typename as u32) as usize])
                {
                    Some(Slot::Raw(raw)) => self.hasher.bytes(7, self.arena.raw(raw)),
                    _ => self.hasher.word(8),
                }
                // what else the representation looks at: whether the object has anything at
                // all, and which key fields it has (the complete-key check)
                if let Some(key_slots) = key_slots {
                    let block = &self.arena.slots[first as usize..(first + width) as usize];
                    let mut mask = block.iter().any(|slot| *slot != Slot::Missing) as u64;
                    let mut bit = 1;
                    for &slot in key_slots {
                        if bit == 64 {
                            self.hasher.word(mask);
                            (mask, bit) = (0, 0);
                        }
                        mask |= ((block[slot as usize] != Slot::Missing) as u64) << bit;
                        bit += 1;
                    }
                    self.hasher.word(mask);
                }
                let requires = self.requires;
                for read in &requires.reads[reads.start as usize..reads.end as usize] {
                    let get = |slot: u16| {
                        (slot != NO_SLOT)
                            .then(|| self.arena.slots[(first + slot as u32) as usize])
                            .filter(|value| *value != Slot::Missing)
                    };
                    match get(read.by_name).or_else(|| get(read.by_alias)) {
                        Some(value) => self.value(value, read.children.clone(), None),
                        // not a value: `value` never hashes 9
                        None => self.hasher.word(9),
                    }
                }
            }
        }
    }
}

/// A small word-at-a-time hash for fingerprints: a folded 128-bit multiply per 8 bytes, like
/// ahash's fallback, without its per-call bookkeeping. Seeded once per process.
struct Mix(u64);

const MIX: u64 = 0x9E37_79B9_7F4A_7C15;

static MIX_SEED: std::sync::LazyLock<u64> =
    std::sync::LazyLock::new(|| ahash::RandomState::new().hash_one(0u64) | 1);

impl Mix {
    fn new() -> Self {
        Self(*MIX_SEED)
    }

    #[inline(always)]
    fn word(&mut self, word: u64) {
        let product = ((self.0 ^ word) as u128).wrapping_mul(MIX as u128);
        self.0 = (product as u64) ^ ((product >> 64) as u64);
    }

    /// A tag and the length in one word, then the bytes 8 at a time. The last few bytes are
    /// read with overlapping loads instead of a copy (with the length known, they still cover
    /// every byte exactly).
    #[inline(always)]
    fn bytes(&mut self, tag: u64, bytes: &[u8]) {
        let len = bytes.len();
        self.word(tag | (len as u64) << 8);
        let load8 = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().expect("8 bytes"));
        let load4 =
            |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().expect("4 bytes")) as u64;
        if len >= 8 {
            let mut at = 0;
            while at + 8 <= len {
                self.word(load8(at));
                at += 8;
            }
            if at < len {
                self.word(load8(len - 8));
            }
        } else if len >= 4 {
            self.word(load4(0) | load4(len - 4) << 32);
        } else if len > 0 {
            self.word(
                bytes[0] as u64 | (bytes[len / 2] as u64) << 8 | (bytes[len - 1] as u64) << 16,
            );
        }
    }

    fn finish(&self) -> u64 {
        let product = (self.0 as u128).wrapping_mul(0xA076_1D64_78BD_642F_u128);
        (product as u64) ^ ((product >> 64) as u64)
    }
}
