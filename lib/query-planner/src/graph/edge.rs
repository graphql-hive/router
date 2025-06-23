use std::fmt::{Debug, Display};

use petgraph::graph::EdgeReference as GraphEdgeReference;

use crate::{
    ast::type_aware_selection::TypeAwareSelection, federation_spec::directives::JoinFieldDirective,
    state::supergraph_state::SubgraphName,
};

#[derive(Debug)]
pub struct EntityMove {
    pub key: String,
    pub requirements: TypeAwareSelection,
    /// Indicates whether the move is to an interface entity.
    ///
    /// Object @key -> Object @key (@interfaceObject)
    ///
    /// Interface @key -> Interface @key
    pub is_interface: bool,
}

#[derive(Debug)]
pub struct InterfaceObjectTypeMove {
    pub object_type_name: String,
    pub requirements: TypeAwareSelection,
}

/// Represent a simple file move
#[derive(Debug)]
pub struct FieldMove {
    pub name: String,
    pub type_name: String,
    pub is_leaf: bool,
    pub is_list: bool,
    pub join_field: Option<JoinFieldDirective>,
    pub requirements: Option<TypeAwareSelection>,
    pub override_from: Option<String>,
}

pub enum Edge {
    /// A special edge between the root Node and then root entry point to the graph
    /// With this helper, you can jump from Query::RootQuery --SomeSubgraph-> Query/SomeSubgraph --> --field--> SomeType/SomeSubgraph
    SubgraphEntrypoint {
        field_names: Vec<String>,
        name: SubgraphName,
    },
    FieldMove(FieldMove),
    EntityMove(EntityMove),
    /// join__implements
    AbstractMove(String),
    /// Represents a special case where going from @interfaceObject
    /// to an object type due to the `__typename` field usage,
    /// or usage of a type condition (fragment),
    /// is not possible as the interface is fake, it's an object type,
    /// so there's no subgraph-level information about object types
    /// implementing the interface,
    /// and resolving the `__typename` in the subgraph
    /// would result in a incorrect value (name of the @interfaceObject type).
    /// This enum variant tells the Query Planner to do an entity call,
    /// to verify the type condition or resolve the __typename.
    InterfaceObjectTypeMove(InterfaceObjectTypeMove),
}

pub type EdgeReference<'a> = GraphEdgeReference<'a, Edge>;

impl Edge {
    pub fn create_entity_move(
        key: &str,
        selection: TypeAwareSelection,
        is_interface: bool,
    ) -> Self {
        Self::EntityMove(EntityMove {
            key: key.to_string(),
            requirements: selection,
            is_interface,
        })
    }

    pub fn create_interface_object_type_move(
        object_type_name: &str,
        selection: TypeAwareSelection,
    ) -> Self {
        Self::InterfaceObjectTypeMove(InterfaceObjectTypeMove {
            object_type_name: object_type_name.to_string(),
            requirements: selection,
        })
    }

    pub fn create_field_move(
        name: String,
        type_name: String,
        is_leaf: bool,
        is_list: bool,
        join_field: Option<JoinFieldDirective>,
        requirements: Option<TypeAwareSelection>,
    ) -> Self {
        let override_from = join_field.as_ref().and_then(|jf| jf.override_value.clone());

        Self::FieldMove(FieldMove {
            name: name.clone(),
            type_name: type_name.clone(),
            is_leaf,
            is_list,
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
            Self::SubgraphEntrypoint { name, .. } => &name.0,
            Self::InterfaceObjectTypeMove(InterfaceObjectTypeMove {
                object_type_name, ..
            }) => object_type_name,
        }
    }

    pub fn requirements(&self) -> Option<&TypeAwareSelection> {
        match self {
            Self::EntityMove(entity_move) => Some(&entity_move.requirements),
            Self::InterfaceObjectTypeMove(m) => Some(&m.requirements),
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
            Edge::SubgraphEntrypoint { name, .. } => write!(f, "{}", name.0),
            Edge::EntityMove(EntityMove { .. }) => write!(f, "🔑"),
            Edge::AbstractMove(_) => write!(f, "🔮"),
            Edge::FieldMove(field_move) => write!(f, "{}", field_move.name),
            Edge::InterfaceObjectTypeMove(m) => write!(f, "🔎 {}", m.object_type_name),
        }?;

        if let Some(reqs) = self.requirements() {
            write!(f, "🧩{}", reqs.selection_set)?
        };

        Ok(())
    }
}

impl Debug for Edge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Edge::SubgraphEntrypoint { name, .. } => write!(f, "subgraph({})", name.0),
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
                    if jf.external {
                        result = result.and_then(|_| write!(f, " @external"));
                    }

                    if let Some(override_from) = &jf.override_value {
                        result =
                            result.and_then(|_| write!(f, " @override(from: {})", override_from));
                    }
                }

                result
            }
            Edge::EntityMove(EntityMove { key, .. }) => {
                write!(f, "🔑 {}", key)
            }
            Edge::AbstractMove(name) => write!(f, "🔮 {}", name),
            Edge::InterfaceObjectTypeMove(m) => write!(f, "🔎 {}", m.object_type_name),
        }
    }
}

impl PartialEq for Edge {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Edge::SubgraphEntrypoint { name: graph_id, .. },
                Edge::SubgraphEntrypoint {
                    name: other_graph_id,
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
            (Edge::InterfaceObjectTypeMove(st), Edge::InterfaceObjectTypeMove(ot)) => {
                st.object_type_name == ot.object_type_name
            }

            _ => false,
        }
    }
}
