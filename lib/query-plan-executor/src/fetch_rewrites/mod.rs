use query_planner::planner::plan_nodes::FetchRewrite;
use serde_json::Value;

use crate::schema_metadata::SchemaMetadata;

pub mod key_renamer;
pub mod value_setter;

pub trait ApplyFetchRewrite {
    fn apply(&self, schema_metadata: &SchemaMetadata, value: &mut Value);
    fn apply_path(&self, schema_metadata: &SchemaMetadata, value: &mut Value, path: &[String]);
}

impl ApplyFetchRewrite for FetchRewrite {
    fn apply(&self, schema_metadata: &SchemaMetadata, value: &mut Value) {
        match self {
            FetchRewrite::KeyRenamer(renamer) => renamer.apply(schema_metadata, value),
            FetchRewrite::ValueSetter(setter) => setter.apply(schema_metadata, value),
        }
    }
    fn apply_path(&self, schema_metadata: &SchemaMetadata, value: &mut Value, path: &[String]) {
        match self {
            FetchRewrite::KeyRenamer(renamer) => renamer.apply_path(schema_metadata, value, path),
            FetchRewrite::ValueSetter(setter) => setter.apply_path(schema_metadata, value, path),
        }
    }
}
