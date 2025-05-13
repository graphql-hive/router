use std::fmt::{Debug, Display};

use petgraph::graph::EdgeReference as GraphEdgeReference;

use crate::federation_spec::directives::JoinFieldDirective;

use super::selection::Selection;

pub struct EntityMove {
    pub key: String,
    pub requirements: Selection,
}

/// Represent a simple file move
pub struct FieldMove {
    pub name: String,
    pub join_field: Option<JoinFieldDirective>,
    pub requirements: Option<Selection>,
    pub override_from: Option<String>,
}

pub enum Edge {
    /// A special edge between the root Node and then root entry point to the graph
    /// With this helper, you can jump from Query::RootQuery --SomeSubgraph-> Query/SomeSubgraph --> --field--> SomeType/SomeSubgraph
    SubgraphEntrypoint {
        field_names: Vec<String>,
        graph_id: String,
    },
    FieldMove(FieldMove),
    EntityMove(EntityMove),
    /// join__implements
    AbstractMove(String),
    // interfaceObject
    // InterfaceObjectMove(String),
}

pub type EdgeReference<'a> = GraphEdgeReference<'a, Edge>;

impl Edge {
    pub fn create_entity_move(key: &str, selection: Selection) -> Self {
        Self::EntityMove(EntityMove {
            key: key.to_string(),
            requirements: selection,
        })
    }

    pub fn create_field_move(
        name: String,
        join_field: Option<JoinFieldDirective>,
        requirements: Option<Selection>,
    ) -> Self {
        let override_from = join_field.as_ref().and_then(|jf| jf.override_value.clone());

        Self::FieldMove(FieldMove {
            name: name.clone(),
            join_field,
            requirements,
            override_from,
        })
    }

    pub fn display_name(&self) -> &str {
        match self {
            Self::FieldMove(FieldMove { name, .. }) => name,
            Self::EntityMove(EntityMove { key, .. }) => key,
            Self::AbstractMove(id) => id,
            Self::SubgraphEntrypoint { graph_id, .. } => graph_id,
        }
    }

    pub fn requirements(&self) -> Option<&Selection> {
        match self {
            Self::EntityMove(entity_move) => Some(&entity_move.requirements),
            Self::FieldMove(field_move) => field_move.requirements.as_ref(),
            _ => None,
        }
    }

    pub fn cost(&self) -> u64 {
        let move_cost = match self {
            Self::FieldMove(FieldMove { .. }) => 1,
            _ => 1000,
        };

        let requirement_cost = match self.requirements() {
            Some(selection) => selection.cost(),
            None => 0,
        };

        move_cost + requirement_cost
    }
}

impl Display for Edge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Edge::SubgraphEntrypoint { graph_id, .. } => write!(f, "{}", graph_id),
            Edge::EntityMove(EntityMove { .. }) => write!(f, "🔑"),
            Edge::AbstractMove(_) => write!(f, "🔮"),
            Edge::FieldMove(field_move) => write!(f, "{}", field_move.name),
        }?;

        match self.requirements() {
            Some(reqs) => write!(f, "🧩{}", reqs)?,
            None => {}
        };

        Ok(())
    }
}

impl Debug for Edge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Edge::SubgraphEntrypoint { graph_id, .. } => write!(f, "subgraph({})", graph_id),
            Edge::FieldMove(FieldMove {
                name, join_field, ..
            }) => {
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
            Edge::EntityMove(EntityMove {
                key,
                requirements: requirement,
            }) => {
                write!(f, "🔑 {} {}", key, requirement)
            }
            Edge::AbstractMove(name) => write!(f, "🔮 {}", name),
        }
    }
}

impl PartialEq for Edge {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Edge::SubgraphEntrypoint { graph_id, .. },
                Edge::SubgraphEntrypoint {
                    graph_id: other_graph_id,
                    ..
                },
            ) => graph_id == other_graph_id,
            (
                Edge::FieldMove(FieldMove {
                    name,
                    join_field: Some(jf1),
                    ..
                }),
                Edge::FieldMove(FieldMove {
                    name: other_name,
                    join_field: Some(jf2),
                    ..
                }),
            ) => {
                // Compare names and directive fields that affect planning
                name == other_name
                    && jf1.requires == jf2.requires
                    && jf1.provides == jf2.provides
                    && jf1.external == jf2.external
                    && jf1.override_value == jf2.override_value
            }

            (
                Edge::FieldMove(FieldMove {
                    name,
                    join_field: None,
                    ..
                }),
                Edge::FieldMove(FieldMove {
                    name: other_name,
                    join_field: None,
                    ..
                }),
            ) => name == other_name,

            (
                Edge::EntityMove(EntityMove { key, .. }),
                Edge::EntityMove(EntityMove { key: other_key, .. }),
            ) => key == other_key,

            (Edge::AbstractMove(name), Edge::AbstractMove(other_name)) => name == other_name,

            _ => false,
        }
    }
}
