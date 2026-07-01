use std::{
    fmt::{Debug, Display},
    sync::Arc,
};

use crate::state::supergraph_state::SubgraphName;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UnionMembersData<'graph> {
    /// Represents the type owning the field
    pub type_name: &'graph str,
    /// Represents the field resolving a union type
    pub field_name: &'graph str,
    /// Represents one concrete member used when a path is narrowed to a single member.
    /// Full reachable member set lives in `possible_members`.
    pub object_type_name: &'graph str,
    /// Represents all union members reachable for the same field in this subgraph.
    pub possible_members: Arc<Vec<&'graph str>>,
    pub provides: Option<u64>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum SubgraphTypeSpecialization<'graph> {
    /// Node was created due to @provides path.
    Provides(u64),
    /// Node represents union tail for a specific field in a specific subgraph.
    ///
    /// For union-returning field moves, we may need a tail that only exposes the
    /// members reachable in the current subgraph. We model that by creating
    /// one specialized tail carrying full member set, then abstract-move edges
    /// from that tail to the concrete member types.
    UnionMembers(UnionMembersData<'graph>),
}

impl<'graph> SubgraphTypeSpecialization<'graph> {
    pub fn union_members_data(&self) -> Option<&UnionMembersData<'graph>> {
        match self {
            SubgraphTypeSpecialization::UnionMembers(data) => Some(data),
            _ => None,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SubgraphType<'graph> {
    pub name: &'graph str,
    pub subgraph: SubgraphName<'graph>,
    pub is_interface_object: bool,
    specialization: Option<SubgraphTypeSpecialization<'graph>>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum Node<'graph> {
    QueryRoot(&'graph str),
    MutationRoot(&'graph str),
    SubscriptionRoot(&'graph str),
    /// Represent an entity type or a scalar living in a specific subgraph
    SubgraphType(SubgraphType<'graph>),
}

impl<'graph> Node<'graph> {
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
                    SubgraphTypeSpecialization::UnionMembers(u) => format!(
                        "{}/{} for {}.{}:{:?}",
                        st.name, st.subgraph.0, u.type_name, u.field_name, u.possible_members
                    ),
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
            Node::SubgraphType(st) => st.name,
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

    pub fn new_node(
        name: &'graph str,
        subgraph: SubgraphName<'graph>,
        is_interface_object: bool,
    ) -> Node<'graph> {
        Node::SubgraphType(SubgraphType {
            name,
            subgraph,
            is_interface_object,
            specialization: None,
        })
    }

    pub fn new_specialized_node(
        name: &'graph str,
        subgraph: SubgraphName<'graph>,
        is_interface_object: bool,
        specialization: SubgraphTypeSpecialization<'graph>,
    ) -> Node<'graph> {
        Node::SubgraphType(SubgraphType {
            name,
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
            Node::SubgraphType(st) => Some(st.subgraph.0),
        }
    }

    pub fn subgraph_type(&self) -> Option<&SubgraphType<'_>> {
        match self {
            Node::SubgraphType(st) => Some(st),
            _ => None,
        }
    }

    pub fn union_members_data(&self) -> Option<&UnionMembersData<'_>> {
        self.subgraph_type()
            .and_then(|st| st.specialization.as_ref())
            .and_then(|s| s.union_members_data())
    }
}

impl Debug for Node<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

impl Display for Node<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}
