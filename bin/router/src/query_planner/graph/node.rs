use std::{
    fmt::{Debug, Display},
    sync::Arc,
};

use crate::query_planner::state::supergraph_state::SubgraphName;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnionMembersData<'graph> {
    /// Represents the type owning the field
    pub type_name: &'graph str,
    /// Represents the field resolving a union type
    pub field_name: &'graph str,
    /// Represents a union member
    pub object_type_name: &'graph str,
    /// Represents all union members reachable for the same field in this subgraph.
    pub possible_members: Arc<Vec<&'graph str>>,
}

// `possible_members` follows from the other fields, so the hash skips it. Equal values
// still hash the same.
impl std::hash::Hash for UnionMembersData<'_> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.type_name.hash(state);
        self.field_name.hash(state);
        self.object_type_name.hash(state);
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum SubgraphTypeSpecialization<'graph> {
    /// Node represents a union member tail for a specific subgraph.
    ///
    /// For union-returning field moves, we may need a tail that only exposes the
    /// members reachable in the current subgraph. We model that by creating
    /// per-member specialized nodes and then abstract-move edges from those tails
    /// to the concrete member types.
    UnionMembers(UnionMembersData<'graph>),
    /// The tail of a field that returns an interface but another type in this subgraph,
    /// `@join__field(graph: A, type: "[Book]")`: named after the interface, with its fields, and
    /// the moves to the types the field can be there only.
    Narrowed(NarrowedFieldData<'graph>),
}

/// See `SubgraphTypeSpecialization::Narrowed`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NarrowedFieldData<'graph> {
    /// The type owning the field
    pub type_name: &'graph str,
    pub field_name: &'graph str,
    /// The type the field returns in this subgraph
    pub to: &'graph str,
}

impl<'graph> SubgraphTypeSpecialization<'graph> {
    pub fn union_members_data(&self) -> Option<&UnionMembersData<'graph>> {
        match self {
            SubgraphTypeSpecialization::UnionMembers(data) => Some(data),
            SubgraphTypeSpecialization::Narrowed(_) => None,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SubgraphType<'graph> {
    pub name: &'graph str,
    pub subgraph: SubgraphName,
    /// `subgraph`'s index in `SupergraphState::subgraph_names`. Set once the graph is built.
    pub(super) subgraph_index: u16,
    pub is_interface_object: bool,
    specialization: Option<SubgraphTypeSpecialization<'graph>>,
    /// What a `@provides` path makes available on top of the plain node, like `{sku}`.
    /// Copies are keyed by it, so two paths that provide the same fields share a node.
    provided: Option<String>,
}

impl SubgraphType<'_> {
    pub fn plain_display_name(&self) -> String {
        format!("{}/{}", self.name, self.subgraph.0)
    }
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
            Node::SubgraphType(st) => {
                // we rely on display_name when it comes to deduplicating nodes (upsert_node),
                // that's why the string produced here should "mimic" hashing
                let name = match &st.specialization {
                    Some(SubgraphTypeSpecialization::UnionMembers(u)) => format!(
                        "{}/{} for {}.{}:{}",
                        st.name, st.subgraph.0, u.type_name, u.field_name, u.object_type_name
                    ),
                    Some(SubgraphTypeSpecialization::Narrowed(n)) => format!(
                        "{}/{} for {}.{} as {}",
                        st.name, st.subgraph.0, n.type_name, n.field_name, n.to
                    ),
                    None => st.plain_display_name(),
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
            Node::SubgraphType(st) => st.name,
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
    pub fn provides_copy(&self, provided: String) -> Node<'graph> {
        let mut copy = self.clone();
        if let Node::SubgraphType(st) = &mut copy {
            st.provided = Some(provided);
        }
        copy
    }

    pub fn new_node(
        name: &'graph str,
        subgraph: SubgraphName,
        is_interface_object: bool,
    ) -> Node<'graph> {
        Node::SubgraphType(SubgraphType {
            name,
            subgraph,
            subgraph_index: 0,
            is_interface_object,
            specialization: None,
            provided: None,
        })
    }

    /// Root types are never interface objects, so this is the same node the field edges use.
    pub fn new_root_node(name: &'graph str, subgraph: SubgraphName) -> Node<'graph> {
        Self::new_node(name, subgraph, false)
    }

    pub fn new_specialized_node(
        name: &'graph str,
        subgraph: SubgraphName,
        is_interface_object: bool,
        specialization: SubgraphTypeSpecialization<'graph>,
    ) -> Node<'graph> {
        Node::SubgraphType(SubgraphType {
            name,
            subgraph,
            subgraph_index: 0,
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

    pub fn subgraph_index(&self) -> Option<u16> {
        self.subgraph_type().map(|st| st.subgraph_index)
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

    /// The type a narrowed field returns in this subgraph, on its tail, see
    /// `SubgraphTypeSpecialization::Narrowed`.
    pub fn narrowed_field_type(&self) -> Option<&str> {
        match self.subgraph_type()?.specialization.as_ref()? {
            SubgraphTypeSpecialization::Narrowed(data) => Some(data.to),
            SubgraphTypeSpecialization::UnionMembers(_) => None,
        }
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
