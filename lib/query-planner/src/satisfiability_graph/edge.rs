use std::fmt::Debug;

use crate::federation_spec::directives::JoinFieldDirective;

pub enum Edge {
    Root, // Root of the graph
    Field {
        name: String,
        join_field: Option<JoinFieldDirective>,
    }, // Field of an entity
    EntityReference(String), // 🔑 Reference to another entity using "@key"
    InterfaceImplementation(String), // 🔮 Interface implementation
}

impl Debug for Edge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Edge::Root => write!(f, ""),
            Edge::Field { name, join_field } => {
                if let Some(join_field) = join_field {
                    let requires = join_field
                        .requires
                        .as_ref()
                        .map_or_else(|| "".to_string(), |v| format!(" @require({})", v));

                    return write!(f, "{}{}", name, requires);
                }

                write!(f, "{}", name)
            }
            Edge::EntityReference(name) => write!(f, "🔑 {}", name),
            Edge::InterfaceImplementation(name) => write!(f, "🔮 {}", name),
        }
    }
}

impl PartialEq for Edge {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Edge::Root, Edge::Root) => true,
            (
                Edge::Field {
                    name,
                    join_field: _,
                },
                Edge::Field {
                    name: other_name,
                    join_field: _,
                },
            ) => name == other_name,
            (Edge::EntityReference(name), Edge::EntityReference(other_name)) => name == other_name,
            (Edge::InterfaceImplementation(name), Edge::InterfaceImplementation(other_name)) => {
                name == other_name
            }
            _ => false,
        }
    }
}
