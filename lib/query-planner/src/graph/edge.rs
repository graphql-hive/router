use std::{
    collections::HashSet,
    fmt::{Debug, Display},
    hash::Hash,
};

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
#[derive(Debug, PartialEq)]
pub struct FieldMove {
    pub name: String,
    pub type_name: String,
    pub is_leaf: bool,
    pub is_list: bool,
    pub join_field: Option<JoinFieldDirective>,
    pub requirements: Option<TypeAwareSelection>,
    pub override_from: Option<String>,
    pub override_label: Option<OverrideLabel>,
    pub overridden_by: Option<(String, Option<OverrideLabel>)>,
}

impl FieldMove {
    pub fn satisfies_override_rules(&self, ctx: &PlannerOverrideContext) -> bool {
        // Field is being progressively overridden by another subgraph
        if let Some((_, Some(label))) = &self.overridden_by {
            return self.check_progressive_override(label, ctx, false);
        }

        // Field is being fully overridden, so it's not resolvable
        if self.overridden_by.is_some() {
            return false;
        }

        // Field is progressively overriding another subgraph
        if let Some(label) = &self.override_label {
            return self.check_progressive_override(label, ctx, true);
        }

        // The field is not involved in an override, so it's always resolvable.
        true
    }

    fn check_progressive_override(
        &self,
        label: &OverrideLabel,
        ctx: &PlannerOverrideContext,
        is_overriding_field: bool,
    ) -> bool {
        match label {
            OverrideLabel::Custom(flag_name) => {
                is_overriding_field == ctx.is_flag_active(flag_name)
            }
            OverrideLabel::Percentage(percentage_in_label) => {
                is_overriding_field == ctx.is_in_range(percentage_in_label)
            }
        }
    }
}

type ActiveFlags = HashSet<String>;

#[derive(Default)]
pub struct PlannerOverrideContext {
    active_flags: ActiveFlags,
    request_percentage_value: Percentage,
}

impl PlannerOverrideContext {
    pub fn new(active_flags: ActiveFlags, request_percentage_value: Percentage) -> Self {
        Self {
            active_flags,
            request_percentage_value,
        }
    }

    pub fn from_percentage(value: f64) -> Self {
        Self {
            active_flags: Default::default(),
            request_percentage_value: (value * (PERCENTAGE_SCALE_FACTOR as f64)) as u64,
        }
    }

    pub fn from_flag(value: String) -> Self {
        Self {
            active_flags: HashSet::from([value]),
            request_percentage_value: 0,
        }
    }

    pub fn is_flag_active(&self, flag_name: &str) -> bool {
        self.active_flags.contains(flag_name)
    }

    pub fn is_in_range(&self, percentage: &Percentage) -> bool {
        &self.request_percentage_value < percentage
    }
}

/// Represents a percentage value
/// as I.F multiplied by 100_000_000
/// Where I is a number between 0 and 100
/// and F represents 8 fraction digits.
/// Min 000.00000000
/// Max 100.00000000
///
/// Why 8? That's the maximum precision composition allows.
pub type Percentage = u64;
pub const PERCENTAGE_SCALE_FACTOR: u64 = 100_000_000;

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum OverrideLabel {
    Custom(String),
    Percentage(Percentage),
}

impl Display for OverrideLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OverrideLabel::Custom(label) => write!(f, "{}", label),
            OverrideLabel::Percentage(percentage) => {
                write!(f, "{}%", percentage / PERCENTAGE_SCALE_FACTOR)
            }
        }
    }
}

pub enum Edge {
    /// A special edge between the root Node and then root entry point to the graph
    /// With this helper, you can jump from Query::RootQuery --SomeSubgraph-> Query/SomeSubgraph --> --field--> SomeType/SomeSubgraph
    SubgraphEntrypoint {
        field_names: Vec<String>,
        name: SubgraphName,
    },
    FieldMove(Box<FieldMove>),
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
        overridden_by: Option<(String, Option<OverrideLabel>)>,
    ) -> Self {
        let override_from = join_field.as_ref().and_then(|jf| jf.override_value.clone());
        let override_label = join_field.as_ref().and_then(|jf| jf.override_label.clone());

        Self::FieldMove(Box::new(FieldMove {
            name: name.clone(),
            type_name: type_name.clone(),
            is_leaf,
            is_list,
            join_field,
            requirements,
            override_from,
            override_label,
            overridden_by,
        }))
    }

    pub fn display_name(&self) -> &str {
        match self {
            Self::FieldMove(fm) => &fm.name,
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
            Self::FieldMove(_) => 1,
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
            Edge::FieldMove(fm) => {
                // Start with the field name
                let mut result = write!(f, "{}", &fm.name);

                // Add requires directive if present
                if let Some(jf) = &fm.join_field {
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
                        let label = jf
                            .override_label
                            .as_ref()
                            .map_or("".to_string(), |label| format!(", label: {}", label));

                        result = result.and_then(|_| {
                            write!(f, " @override(from: {}{})", override_from, label)
                        });
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
            (Edge::FieldMove(fm1), Edge::FieldMove(fm2)) => fm1 == fm2,
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
