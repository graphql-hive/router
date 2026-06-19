use std::fmt::{Debug, Display};

use crate::state::supergraph_state::SubgraphName;

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct UnionMembersData<'a> {
    pub type_name: &'a str,
    pub field_name: &'a str,
    pub object_type_name: &'a str,
    pub possible_members: std::sync::Arc<Vec<&'a str>>,
    pub provides: Option<u64>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum SubgraphTypeSpecialization<'a> {
    Provides(u64),
    UnionMembers(UnionMembersData<'a>),
}

impl<'a> SubgraphTypeSpecialization<'a> {
    pub fn union_members_data(&self) -> Option<&UnionMembersData<'a>> {
        match self {
            SubgraphTypeSpecialization::UnionMembers(data) => Some(data),
            _ => None,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SubgraphType<'a> {
    pub name: &'a str,
    pub subgraph: SubgraphName<'a>,
    pub is_interface_object: bool,
    pub(crate) specialization: Option<SubgraphTypeSpecialization<'a>>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum Node<'a> {
    QueryRoot(&'a str),
    MutationRoot(&'a str),
    SubscriptionRoot(&'a str),
    SubgraphType(SubgraphType<'a>),
}

impl<'a> Node<'a> {
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

    pub fn new_node(name: &'a str, subgraph: SubgraphName<'a>, is_interface_object: bool) -> Node<'a> {
        Node::SubgraphType(SubgraphType {
            name,
            subgraph,
            is_interface_object,
            specialization: None,
        })
    }

    pub fn new_specialized_node(
        name: &'a str,
        subgraph: SubgraphName<'a>,
        is_interface_object: bool,
        specialization: SubgraphTypeSpecialization<'a>,
    ) -> Node<'a> {
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
            Node::SubgraphType(st) => Some(&st.subgraph.0),
        }
    }

    pub fn subgraph_type(&self) -> Option<&SubgraphType<'a>> {
        match self {
            Node::SubgraphType(st) => Some(st),
            _ => None,
        }
    }

    pub fn union_members_data(&self) -> Option<&UnionMembersData<'a>> {
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
