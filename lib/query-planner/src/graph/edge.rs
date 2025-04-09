use std::fmt::Debug;

use petgraph::graph::EdgeIndex;

use crate::federation_spec::directives::JoinFieldDirective;

use super::selection::SelectionNode;

pub type EdgePair<'a> = (&'a Edge, EdgeIndex);

pub enum Edge {
    /// A special edge between the root Node and then root entry point to the graph
    /// With this helper, you can jump from Query::RootQuery --field-> Query/SomeSubgraph --> --field--> SomeType/SomeSubgraph
    RootEntrypoint {
        field_name: String,
    },
    /// Represent a simple file move
    FieldMove {
        name: String,
        join_field: Option<JoinFieldDirective>,
        requires: Option<SelectionNode>,
        override_from: Option<String>,
    },
    EntityMove(String),
    /// join__implements
    AbstractMove(String),
    // interfaceObject
    // InterfaceObjectMove(String),
}

impl Edge {
    /// Helper to create a Field edge from a field name and join directive
    pub fn create_field_move(
        name: String,
        join_field: Option<JoinFieldDirective>,
        field_type: &str,
    ) -> Self {
        let requires = join_field.as_ref().and_then(|jf| jf.requires.clone());
        let override_from = join_field.as_ref().and_then(|jf| jf.override_value.clone());

        Self::FieldMove {
            name: name.clone(),
            join_field,
            requires: requires.map(|s| SelectionNode::parse_field_selection(s, &name, field_type)),
            override_from,
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::FieldMove { name, .. } => name,
            Self::EntityMove(id) => id,
            Self::AbstractMove(id) => id,
            Self::RootEntrypoint { field_name } => field_name,
        }
    }

    /// Gets the requirements as a string, if any
    pub fn requirements(&self) -> Option<&SelectionNode> {
        match self {
            Self::FieldMove { requires, .. } => requires.as_ref(),
            _ => None,
        }
    }

    pub fn cost(&self) -> u64 {
        match self {
            Self::FieldMove { .. } => 1,
            _ => 10,
        }
    }
}

impl Debug for Edge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Edge::RootEntrypoint { field_name } => write!(f, "root({})", field_name),

            Edge::FieldMove {
                name, join_field, ..
            } => {
                // Start with the field name
                let mut result = write!(f, "{}", name);

                // Add requires directive if present
                if let Some(jf) = join_field {
                    if let Some(req) = &jf.requires {
                        result = result.and_then(|_| write!(f, " @requires({})", req));
                    }

                    // Add provides directive if present
                    if jf.provides.is_some() {
                        result = result.and_then(|_| write!(f, " @provides"));
                    }

                    // Add other relevant directives like external, override, etc.
                    if jf.external.unwrap_or(false) {
                        result = result.and_then(|_| write!(f, " @external"));
                    }

                    if let Some(override_from) = &jf.override_value {
                        result =
                            result.and_then(|_| write!(f, " @override(from: {})", override_from));
                    }
                }

                result
            }
            Edge::EntityMove(name) => write!(f, "🔑 {}", name),
            Edge::AbstractMove(name) => write!(f, "🔮 {}", name),
        }
    }
}

impl PartialEq for Edge {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Edge::RootEntrypoint { field_name },
                Edge::RootEntrypoint {
                    field_name: other_field_name,
                },
            ) => field_name == other_field_name,
            (
                Edge::FieldMove {
                    name,
                    join_field: Some(jf1),
                    ..
                },
                Edge::FieldMove {
                    name: other_name,
                    join_field: Some(jf2),
                    ..
                },
            ) => {
                // Compare names and directive fields that affect planning
                name == other_name
                    && jf1.requires == jf2.requires
                    && jf1.provides == jf2.provides
                    && jf1.external == jf2.external
                    && jf1.override_value == jf2.override_value
            }

            (
                Edge::FieldMove {
                    name,
                    join_field: None,
                    ..
                },
                Edge::FieldMove {
                    name: other_name,
                    join_field: None,
                    ..
                },
            ) => name == other_name,

            (Edge::EntityMove(name), Edge::EntityMove(other_name)) => name == other_name,

            (Edge::AbstractMove(name), Edge::AbstractMove(other_name)) => name == other_name,

            _ => false,
        }
    }
}
