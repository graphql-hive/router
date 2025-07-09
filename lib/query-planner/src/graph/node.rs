use std::fmt::{Debug, Display};

use crate::state::supergraph_state::SubgraphName;

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct UnionSubsetData {
    /// Represents the type owning the field
    pub type_name: String,
    /// Represents the field resolving a union type
    pub field_name: String,
    /// Represents a union member
    pub object_type_name: String,
    pub provides: Option<u64>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum SubgraphTypeSpecialization {
    /// Node was created due to @provides path.
    Provides(u64),
    /// Node is part of the union intersection.
    /// When dealing with unions,
    /// we need to point field-move edge's tails (union)
    /// to a subset of object types.
    /// We do it by creating a new Node for each edge's tail (union),
    /// and from that tail we create an abstract-move edges to the object types.
    /// (type_name, field_name, union_member_name)
    UnionSubset(UnionSubsetData),
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SubgraphType {
    pub name: String,
    pub subgraph: SubgraphName,
    pub is_interface_object: bool,
    specialization: Option<SubgraphTypeSpecialization>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum Node {
    QueryRoot(String),
    MutationRoot(String),
    SubscriptionRoot(String),
    /// Represent an entity type or a scalar living in a specific subgraph
    SubgraphType(SubgraphType),
}

impl Node {
    pub fn display_name(&self) -> String {
        match self {
            Node::QueryRoot(name) => format!("root({})", name),
            Node::MutationRoot(name) => format!("root({})", name),
            Node::SubscriptionRoot(name) => format!("root({})", name),
            Node::SubgraphType(st) => match &st.specialization {
                Some(spec) => match spec {
                    SubgraphTypeSpecialization::Provides(provides_id) => {
                        format!("{}/{}/{}", st.name, st.subgraph.0, provides_id)
                    }
                    SubgraphTypeSpecialization::UnionSubset(u) => {
                        // we rely on display_name when it comes to deduplicating nodes (upsert_node),
                        // that's why the string produced here should "mimic" hashing
                        format!(
                            "{}/{} for {}.{}:{}",
                            st.name, st.subgraph.0, u.type_name, u.field_name, u.object_type_name
                        )
                    }
                },
                None => format!("{}/{}", st.name, st.subgraph.0),
            },
        }
    }

    pub fn is_using_provides(&self) -> bool {
        match self {
            Node::QueryRoot(_) => false,
            Node::MutationRoot(_) => false,
            Node::SubscriptionRoot(_) => false,
            Node::SubgraphType(st) => st
                .specialization
                .as_ref()
                .is_some_and(|spec| matches!(spec, SubgraphTypeSpecialization::Provides(_))),
        }
    }

    pub fn new_node(name: &str, subgraph: SubgraphName, is_interface_object: bool) -> Node {
        Node::SubgraphType(SubgraphType {
            name: name.to_string(),
            subgraph,
            is_interface_object,
            specialization: None,
        })
    }

    pub fn new_specialized_node(
        name: &str,
        subgraph: SubgraphName,
        is_interface_object: bool,
        specialization: SubgraphTypeSpecialization,
    ) -> Node {
        Node::SubgraphType(SubgraphType {
            name: name.to_string(),
            subgraph,
            is_interface_object,
            specialization: Some(specialization),
        })
    }

    pub fn graph_id(&self) -> Option<&str> {
        match self {
            Node::QueryRoot(_) => None,
            Node::MutationRoot(_) => None,
            Node::SubscriptionRoot(_) => None,
            Node::SubgraphType(st) => Some(&st.subgraph.0),
        }
    }
}

impl Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

impl Display for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}
