use std::fmt::{Debug, Display};

use crate::state::supergraph_state::SubgraphName;

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SubgraphType {
    pub name: String,
    pub subgraph: SubgraphName,
    provides_identifier: Option<u64>,
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
            Node::SubgraphType(st) => match st.provides_identifier {
                Some(provides_id) => format!("{}/{}/{}", st.name, st.subgraph.0, provides_id),
                None => format!("{}/{}", st.name, st.subgraph.0),
            },
        }
    }

    pub fn is_using_provides(&self) -> bool {
        match self {
            Node::QueryRoot(_) => false,
            Node::MutationRoot(_) => false,
            Node::SubscriptionRoot(_) => false,
            Node::SubgraphType(st) => st.provides_identifier.is_some(),
        }
    }

    pub fn new_node(name: &str, subgraph: SubgraphName) -> Node {
        Node::SubgraphType(SubgraphType {
            name: name.to_string(),
            subgraph,
            provides_identifier: None,
        })
    }

    pub fn new_provides_node(name: &str, subgraph: SubgraphName, provides_id: u64) -> Node {
        Node::SubgraphType(SubgraphType {
            name: name.to_string(),
            subgraph,
            provides_identifier: Some(provides_id),
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
