use std::borrow::Cow;

use hive_router_query_planner::planner::plan_nodes::{
    FetchNodePathSegment, FetchRewrite, KeyRenamer, ValueSetter,
};

use crate::{
    introspection::schema::PossibleTypes, response::value::Value,
    utils::consts::TYPENAME_FIELD_NAME,
};

pub trait FetchRewriteExt {
    fn rewrite<'a>(&'a self, possible_types: &PossibleTypes, value: &mut Value<'a>);
}

impl FetchRewriteExt for FetchRewrite {
    fn rewrite<'a>(&'a self, possible_types: &PossibleTypes, value: &mut Value<'a>) {
        match self {
            FetchRewrite::KeyRenamer(key_renamer) => key_renamer.apply(possible_types, value),
            FetchRewrite::ValueSetter(value_setter) => value_setter.apply(possible_types, value),
        }
    }
}

trait RewriteApplier {
    fn apply<'a>(&'a self, possible_types: &PossibleTypes, value: &mut Value<'a>);
    fn apply_path<'a>(
        &'a self,
        possible_types: &PossibleTypes,
        value: &mut Value<'a>,
        path: &'a [FetchNodePathSegment],
    );
}

impl RewriteApplier for KeyRenamer {
    fn apply<'a>(&'a self, possible_types: &PossibleTypes, value: &mut Value<'a>) {
        self.apply_path(possible_types, value, &self.path)
    }
    fn apply_path<'a>(
        &'a self,
        possible_types: &PossibleTypes,
        value: &mut Value<'a>,
        path: &'a [FetchNodePathSegment],
    ) {
        let current_segment = &path[0];
        let remaining_path = &path[1..];
        match value {
            Value::Array(arr) => {
                for item in arr {
                    self.apply_path(possible_types, item, path);
                }
            }
            Value::Object(obj) => match current_segment {
                FetchNodePathSegment::TypenameEquals(type_condition) => {
                    let type_name = obj
                        .iter()
                        .find(|(key, _)| key == &TYPENAME_FIELD_NAME)
                        .and_then(|(_, val)| val.as_str())
                        .unwrap_or(type_condition);
                    if possible_types.entity_satisfies_type_condition(type_name, type_condition) {
                        self.apply_path(possible_types, value, remaining_path)
                    }
                }
                FetchNodePathSegment::Key(field_name) => {
                    if remaining_path.is_empty() {
                        if field_name != &self.rename_key_to {
                            if let Some((key, _)) =
                                obj.iter_mut().find(|(key, _)| key == field_name)
                            {
                                *key = self.rename_key_to.as_str()
                            }
                        }
                    } else if let Some(data) = obj.iter_mut().find(|r| r.0 == field_name) {
                        self.apply_path(possible_types, &mut data.1, remaining_path)
                    }
                }
            },
            // If the value is not an object or an array, we can't apply the rewrite.
            _ => (),
        }
    }
}

impl RewriteApplier for ValueSetter {
    fn apply<'a>(&'a self, possible_types: &PossibleTypes, data: &mut Value<'a>) {
        self.apply_path(possible_types, data, &self.path)
    }

    fn apply_path<'a>(
        &'a self,
        possible_types: &PossibleTypes,
        data: &mut Value<'a>,
        path: &'a [FetchNodePathSegment],
    ) {
        if path.is_empty() {
            *data = Value::String(Cow::Borrowed(self.set_value_to.as_str()));
            return;
        }

        match data {
            Value::Array(arr) => {
                for data in arr {
                    self.apply_path(possible_types, data, path);
                }
            }
            Value::Object(map) => {
                let current_segment = &path[0];
                let remaining_path = &path[1..];

                match current_segment {
                    FetchNodePathSegment::TypenameEquals(type_condition) => {
                        let type_name = map
                            .iter()
                            .find(|(key, _)| key == &TYPENAME_FIELD_NAME)
                            .and_then(|(_, val)| val.as_str())
                            .unwrap_or(type_condition);
                        if possible_types.entity_satisfies_type_condition(type_name, type_condition)
                        {
                            self.apply_path(possible_types, data, remaining_path)
                        }
                    }
                    FetchNodePathSegment::Key(field_name) => {
                        if let Some(data) = map.iter_mut().find(|r| r.0 == field_name) {
                            self.apply_path(possible_types, &mut data.1, remaining_path)
                        }
                    }
                }
            }
            // If the value is not an object or an array, we can't apply the rewrite.
            _ => (),
        }
    }
}
