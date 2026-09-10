use std::collections::{HashMap, HashSet};

use graphql_tools::parser::schema::{Document as SchemaDocument, TypeDefinition};

pub trait JsonView: Copy {
    fn is_null(self) -> bool;
    fn is_object(self) -> bool;
    fn is_array(self) -> bool;
    fn visit_entries(self, visit: &mut dyn FnMut(&str, Self));
    fn visit_items(self, visit: &mut dyn FnMut(Self));
}

pub trait VariablesPayload {
    type Value<'a>: JsonView
    where
        Self: 'a;

    fn get(&self, key: &str) -> Option<Self::Value<'_>>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariablePlanEntry {
    pub var_name: String,
    pub type_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariableUsageMarker {
    pub var_name: String,
    pub coordinate: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VariablesExtractionPlan {
    pub entries: Vec<VariablePlanEntry>,
    pub markers: Vec<VariableUsageMarker>,
}

impl VariablesExtractionPlan {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.markers.is_empty()
    }

    pub fn extract<P: VariablesPayload>(
        &self,
        variables: Option<&P>,
        schema: &SchemaDocument<'static, String>,
    ) -> Vec<String> {
        let mut out: HashSet<String> = HashSet::new();

        for marker in &self.markers {
            if value_exists(variables.and_then(|p| p.get(&marker.var_name))) {
                out.insert(format!("{}!", marker.coordinate));
            }
        }

        for entry in &self.entries {
            let value = variables.and_then(|p| p.get(&entry.var_name));
            collect_variable(&entry.type_name, value, schema, &mut out);
        }

        out.into_iter().collect()
    }
}

fn value_exists<V: JsonView>(value: Option<V>) -> bool {
    matches!(value, Some(v) if !v.is_null())
}

fn is_builtin_scalar(type_name: &str) -> bool {
    matches!(type_name, "String" | "Int" | "Float" | "Boolean" | "ID")
}

fn collect_variable<V: JsonView>(
    type_name: &str,
    value: Option<V>,
    schema: &SchemaDocument<'static, String>,
    out: &mut HashSet<String>,
) {
    if let Some(v) = value {
        if v.is_array() {
            v.visit_items(&mut |item| collect_variable(type_name, Some(item), schema, out));
            return;
        }
    }

    match schema.type_by_name(type_name) {
        Some(TypeDefinition::InputObject(input_object)) => match value {
            Some(v) if v.is_object() => {
                v.visit_entries(&mut |field_name, field_value| {
                    if let Some(field_def) =
                        input_object.fields.iter().find(|f| f.name == field_name)
                    {
                        let coordinate = format!("{}.{}", input_object.name, field_name);
                        if !field_value.is_null() {
                            out.insert(format!("{}!", coordinate));
                        }
                        out.insert(coordinate);

                        let child_type = field_def.value_type.inner_type();
                        collect_variable(child_type, Some(field_value), schema, out);
                    }
                });
            }
            _ => {
                // Empty value (null / absent / non-object): the bare input type
                // name is marked as used, with no fields.
                out.insert(input_object.name.clone());
            }
        },
        Some(TypeDefinition::Enum(enum_type)) => {
            for value in &enum_type.values {
                out.insert(format!("{}.{}", enum_type.name, value.name));
            }
        }
        Some(TypeDefinition::Scalar(scalar)) => {
            out.insert(scalar.name.clone());
        }
        _ => {
            if is_builtin_scalar(type_name) {
                out.insert(type_name.to_string());
            }
        }
    }
}

#[derive(Clone, Copy)]
pub struct SerdeJsonView<'a>(pub &'a serde_json::Value);

impl JsonView for SerdeJsonView<'_> {
    fn is_null(self) -> bool {
        self.0.is_null()
    }

    fn is_object(self) -> bool {
        self.0.is_object()
    }

    fn is_array(self) -> bool {
        self.0.is_array()
    }

    fn visit_entries(self, visit: &mut dyn FnMut(&str, Self)) {
        if let Some(map) = self.0.as_object() {
            for (key, value) in map {
                visit(key.as_str(), SerdeJsonView(value));
            }
        }
    }

    fn visit_items(self, visit: &mut dyn FnMut(Self)) {
        if let Some(items) = self.0.as_array() {
            for value in items {
                visit(SerdeJsonView(value));
            }
        }
    }
}

impl VariablesPayload for serde_json::Map<String, serde_json::Value> {
    type Value<'a> = SerdeJsonView<'a>;

    fn get(&self, key: &str) -> Option<Self::Value<'_>> {
        serde_json::Map::get(self, key).map(SerdeJsonView)
    }
}

impl VariablesPayload for serde_json::Value {
    type Value<'a> = SerdeJsonView<'a>;

    fn get(&self, key: &str) -> Option<Self::Value<'_>> {
        self.as_object()
            .and_then(|map| map.get(key))
            .map(SerdeJsonView)
    }
}

#[derive(Clone, Copy)]
pub struct SonicJsonView<'a>(pub &'a sonic_rs::Value);

impl JsonView for SonicJsonView<'_> {
    fn is_null(self) -> bool {
        sonic_rs::JsonValueTrait::is_null(self.0)
    }

    fn is_object(self) -> bool {
        sonic_rs::JsonValueTrait::is_object(self.0)
    }

    fn is_array(self) -> bool {
        sonic_rs::JsonValueTrait::is_array(self.0)
    }

    fn visit_entries(self, visit: &mut dyn FnMut(&str, Self)) {
        if let Some(object) = sonic_rs::JsonContainerTrait::as_object(self.0) {
            for (key, value) in object.iter() {
                visit(key, SonicJsonView(value));
            }
        }
    }

    fn visit_items(self, visit: &mut dyn FnMut(Self)) {
        if let Some(array) = sonic_rs::JsonContainerTrait::as_array(self.0) {
            for value in array.iter() {
                visit(SonicJsonView(value));
            }
        }
    }
}

impl VariablesPayload for HashMap<String, sonic_rs::Value> {
    type Value<'a> = SonicJsonView<'a>;

    fn get(&self, key: &str) -> Option<Self::Value<'_>> {
        HashMap::get(self, key).map(SonicJsonView)
    }
}
