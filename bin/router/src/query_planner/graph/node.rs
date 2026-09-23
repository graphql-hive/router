use std::fmt::{Debug, Display};

use crate::query_planner::state::supergraph_state::SubgraphName;

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct UnionMembersData {
    /// Represents the type owning the field
    pub type_name: String,
    /// Represents the field resolving a union type
    pub field_name: String,
    /// Represents a union member
    pub object_type_name: String,
    /// Represents all union members reachable for the same field in this subgraph.
    pub possible_members: Vec<String>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum SubgraphTypeSpecialization {
    /// Root/entrypoint type for a subgraph.
    Root,
    /// Node represents a union member tail for a specific subgraph.
    ///
    /// For union-returning field moves, we may need a tail that only exposes the
    /// members reachable in the current subgraph. We model that by creating
    /// per-member specialized nodes and then abstract-move edges from those tails
    /// to the concrete member types.
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
    specialization: Option<SubgraphTypeSpecialization>,
    /// What a `@provides` path makes available on top of the plain node, like `{sku}`.
    /// Copies are keyed by it, so two paths that provide the same fields share a node.
    provided: Option<String>,
}

impl SubgraphType {
    pub fn plain_display_name(&self) -> String {
        format!("{}/{}", self.name, self.subgraph.0)
    }
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
            Node::SubgraphType(st) => {
                // we rely on display_name when it comes to deduplicating nodes (upsert_node),
                // that's why the string produced here should "mimic" hashing
                let name = match &st.specialization {
                    Some(SubgraphTypeSpecialization::UnionMembers(u)) => format!(
                        "{}/{} for {}.{}:{}",
                        st.name, st.subgraph.0, u.type_name, u.field_name, u.object_type_name
                    ),
                    Some(SubgraphTypeSpecialization::Root) | None => st.plain_display_name(),
                };
                match &st.provided {
                    Some(provided) => format!("{}{}", name, provided),
                    None => name,
                }
            }
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
            Node::SubgraphType(st) => st.provided.is_some(),
        }
    }

    /// A copy of this node for a `@provides` path that makes `provided` available.
    /// Everything else stays, so a union member tail is still a union member tail.
    pub fn provides_copy(&self, provided: String) -> Node {
        let mut copy = self.clone();
        if let Node::SubgraphType(st) = &mut copy {
            st.provided = Some(provided);
        }
        copy
    }

    pub fn new_node(name: &str, subgraph: SubgraphName, is_interface_object: bool) -> Node {
        Node::SubgraphType(SubgraphType {
            name: name.to_string(),
            subgraph,
            is_interface_object,
            specialization: None,
            provided: None,
        })
    }

    pub fn new_root_node(name: &str, subgraph: SubgraphName) -> Node {
        Node::SubgraphType(SubgraphType {
            name: name.to_string(),
            subgraph,
            is_interface_object: false,
            specialization: Some(SubgraphTypeSpecialization::Root),
            provided: None,
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
            provided: None,
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
