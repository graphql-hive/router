/// The "state" representation of a fetch step and fetch graph.
///
/// We split the work we do on the FetchGraph to to distinct parts:
///
/// 1. Graph building: mainly happens in `fetch_graph.rs` and is using `SingleTypeFetchStep` as state.
///    A step is built with a single type in mind. The one exception is an entity call that takes
///    the objects of several types at one spot, like `search.@|[Book|Movie]`: what goes in it says
///    which type it's for, see `add_output_at`.
/// 2. Graph optimization: mainly happens in `fetch/optimize/` files, and using `MultiTypeFetchStep` as state.
///    This ensures that the step is optimized with multiple types in mind.
///
/// With this setup, we can ensure that some parts of the logic/capabilities that can be performed on a selection set or a step are limited,
/// and either scoped to a single type or a multi-type context.

#[derive(Debug, Clone)]
pub struct SingleTypeFetchStep;

#[derive(Debug, Clone)]
pub struct MultiTypeFetchStep;
