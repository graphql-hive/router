use graphql_parser_hive_fork::{
    parse_schema,
    schema::{Definition, Directive, ObjectType, TypeDefinition, Value},
};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Error, Debug)]
#[error("supergraph parse error: {}", _0)]
pub struct ParseError(String);

#[derive(Debug)]
pub struct SupergraphIR {
    pub type_definitions: HashMap<String, SuperTypeDefinition>,
}

impl SupergraphIR {
    pub fn new() -> SupergraphIR {
        SupergraphIR {
            type_definitions: HashMap::new(),
        }
    }

    pub fn add_object_type<'a>(&mut self, object_type: ObjectType<'a, String>) {
        let fields: Vec<SuperFieldDefinition> = object_type
            .fields
            .iter()
            .map(|field| SuperFieldDefinition {
                name: field.name.clone(),
                field_type: field.field_type.to_string(),
                join: Some(get_join_field_directives(field.directives.clone())),
            })
            .collect();

        let directives = object_type.directives.clone();
        self.type_definitions.insert(
            object_type.name.clone(),
            SuperTypeDefinition::Object(SuperObjectTypeDefinition {
                name: object_type.name.clone(),
                fields,
                join: Some(get_join_type_directives(directives)),
            }),
        );
    }
}

#[derive(Debug)]
pub enum SuperTypeDefinition {
    Object(SuperObjectTypeDefinition),
}

#[derive(Debug)]
pub struct SuperObjectTypeDefinition {
    pub name: String,
    pub fields: Vec<SuperFieldDefinition>,
    pub join: Option<Vec<JoinType>>,
}

#[derive(Debug)]
pub struct JoinType {
    graph: String,
    key: Option<String>,
    extension: bool,
    resolvable: bool,
    is_interface_object: bool,
}

#[derive(Debug)]
pub struct JoinField {
    graph: Option<String>,
    requires: Option<String>,
    provides: Option<String>,
    type_in_graph: Option<String>,
    external: bool,
    override_value: Option<String>,
    used_overridden: bool,
}

#[derive(Debug)]
pub struct SuperFieldDefinition {
    pub name: String,
    pub field_type: String,
    pub join: Option<Vec<JoinField>>,
}

fn get_join_type_directives<'a>(directives: Vec<Directive<'a, String>>) -> Vec<JoinType> {
    let mut join_types: Vec<JoinType> = Vec::new();

    for directive in directives {
        if directive.name.ne("join__type") {
            continue;
        }

        let mut graph: Option<String> = None;
        let mut key: Option<String> = None;
        let mut extension: bool = false;
        let mut resolvable: bool = true;
        let mut is_interface_object: bool = false;

        // iterate over arguments and set values
        for (arg_name, arg_value) in directive.arguments {
            if arg_name.eq("graph") {
                match arg_value {
                    Value::String(value) => graph = Some(value),
                    Value::Enum(value) => graph = Some(value),
                    _ => {}
                }
            } else if arg_name.eq("key") {
                match arg_value {
                    Value::String(value) => key = Some(value),
                    _ => {}
                }
            } else if arg_name.eq("extension") {
                match arg_value {
                    Value::Boolean(value) => extension = value,
                    _ => {}
                }
            } else if arg_name.eq("resolvable") {
                match arg_value {
                    Value::Boolean(value) => resolvable = value,
                    _ => {}
                }
            } else if arg_name.eq("is_interface_object") {
                match arg_value {
                    Value::Boolean(value) => is_interface_object = value,
                    _ => {}
                }
            }
        }

        if let Some(graph) = graph {
            join_types.push(JoinType {
                graph,
                key,
                extension,
                resolvable,
                is_interface_object,
            });
        }
    }

    join_types
}

fn get_join_field_directives<'a>(directives: Vec<Directive<'a, String>>) -> Vec<JoinField> {
    let mut join_fields: Vec<JoinField> = Vec::new();

    for directive in directives {
        if directive.name.ne("join__field") {
            continue;
        }

        let mut graph: Option<String> = None;
        let mut requires: Option<String> = None;
        let mut provides: Option<String> = None;
        let mut type_in_graph: Option<String> = None;
        let mut external: bool = false;
        let mut override_value: Option<String> = None;
        let mut used_overridden: bool = false;

        // iterate over arguments and set values
        for (arg_name, arg_value) in directive.arguments {
            if arg_name.eq("graph") {
                match arg_value {
                    Value::String(value) => graph = Some(value),
                    Value::Enum(value) => graph = Some(value),
                    _ => {}
                }
            } else if arg_name.eq("requires") {
                match arg_value {
                    Value::String(value) => requires = Some(value),
                    _ => {}
                }
            } else if arg_name.eq("provides") {
                match arg_value {
                    Value::String(value) => provides = Some(value),
                    _ => {}
                }
            } else if arg_name.eq("type") {
                match arg_value {
                    Value::String(value) => type_in_graph = Some(value),
                    _ => {}
                }
            } else if arg_name.eq("external") {
                match arg_value {
                    Value::Boolean(value) => external = value,
                    _ => {}
                }
            } else if arg_name.eq("override") {
                match arg_value {
                    Value::String(value) => override_value = Some(value),
                    _ => {}
                }
            } else if arg_name.eq("usedOverridden") {
                match arg_value {
                    Value::Boolean(value) => used_overridden = value,
                    _ => {}
                }
            }
        }

        join_fields.push(JoinField {
            graph,
            requires,
            provides,
            type_in_graph,
            external,
            override_value,
            used_overridden,
        });
    }

    join_fields
}

pub fn parse_supergraph<'a>(sdl: &'a str) -> Result<SupergraphIR, ParseError> {
    let schema = parse_schema::<'a, String>(sdl).map_err(|e| ParseError(e.to_string()))?;

    let mut supergraph = SupergraphIR::new();

    for def in schema.definitions {
        match def {
            Definition::TypeDefinition(type_def) => match type_def {
                TypeDefinition::Object(object_type_def) => {
                    supergraph.add_object_type(object_type_def);
                }
                _ => {}
            },
            _ => {}
        }
    }

    Ok(supergraph)
}
