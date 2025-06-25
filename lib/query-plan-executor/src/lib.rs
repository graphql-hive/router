pub mod deep_merge;
pub mod execution_context;
pub mod execution_request;
pub mod execution_result;
pub mod executors;
pub mod fetch_rewrites;
pub mod introspection;
pub mod nodes;
pub mod projection;
pub mod schema_metadata;
pub mod traverse_path;
pub mod validation;
mod value_from_ast;
pub mod variables;

const TYPENAME_FIELD: &str = "__typename";

#[cfg(test)]
mod tests;
