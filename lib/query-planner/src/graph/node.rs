use std::{
    fmt::{Debug, Display},
    sync::Arc,
};

use crate::state::supergraph_state::SubgraphName;

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct UnionMembersData {
    /// Represents the type owning the field
    pub type_name: String,
    /// Represents the field resolving a union type
    pub field_name: String,
    /// Representative member for compatibility with path context before an AbstractMove.
    pub object_type_name: String,
    /// Represents all union members reachable for the same field in this subgraph.
    pub possible_members: Arc<Vec<String>>,
    pub provides: Option<u64>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum SubgraphTypeSpecialization {
    /// Node was created due to @provides path.
    Provides(u64),
    /// Node represents a union member tail for a specific subgraph.
    ///
    /// For union-returning field moves, we may need a tail that only exposes the
    /// members reachable in the current subgraph.
    UnionMembers(UnionMembersData),
}

impl SubgraphTypeSpecialization {
    pub fn union_members_data(&self) -> Option<&UnionMembersData> {
        match self {
            SubgraphTypeSpecialization::UnionMembers(data) => Some(data),
            _ => None,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SubgraphType {
    pub name: String,
    pub subgraph: SubgraphName,
    pub is_interface_object: bool,
    pub(crate) specialization: Option<SubgraphTypeSpecialization>,
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
                    SubgraphTypeSpecialization::UnionMembers(u) => {
                        // we rely on display_name when it comes to deduplicating nodes (upsert_node),
                        // that's why the string produced here should "mimic" hashing
                        format!(
                            "{}/{} for {}.{}:{:?}",
                            st.name, st.subgraph.0, u.type_name, u.field_name, u.possible_members
                        )
                    }
                },
                None => format!("{}/{}", st.name, st.subgraph.0),
            },
        }
    }

    pub fn name_str(&self) -> &str {
        match self {
            Node::QueryRoot(name) => name,
            Node::MutationRoot(name) => name,
            Node::SubscriptionRoot(name) => name,
            Node::SubgraphType(st) => &st.name,
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

    pub fn subgraph_type(&self) -> Option<&SubgraphType> {
        match self {
            Node::SubgraphType(st) => Some(st),
            _ => None,
        }
    }

    pub fn union_members_data(&self) -> Option<&UnionMembersData> {
        self.subgraph_type()
            .and_then(|st| st.specialization.as_ref())
            .and_then(|s| s.union_members_data())
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
