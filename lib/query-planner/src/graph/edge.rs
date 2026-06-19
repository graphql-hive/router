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

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct EntityMove<'a> {
    pub key: &'a str,
    pub requirements: std::sync::Arc<TypeAwareSelection<'a>>,
    pub is_interface: bool,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct InterfaceObjectTypeMove<'a> {
    pub object_type_name: &'a str,
    pub requirements: std::sync::Arc<TypeAwareSelection<'a>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FieldMove<'a> {
    pub name: &'a str,
    pub type_name: &'a str,
    pub is_leaf: bool,
    pub is_list: bool,
    pub join_field: Option<JoinFieldDirective>,
    pub requirements: Option<std::sync::Arc<TypeAwareSelection<'a>>>,
    pub override_from: Option<String>,
    pub override_label: Option<OverrideLabel>,
    pub overridden_by: Option<(String, Option<OverrideLabel>)>,
}

impl FieldMove<'_> {
    pub fn satisfies_override_rules(&self, ctx: &PlannerOverrideContext) -> bool {
        if let Some((_, Some(label))) = &self.overridden_by {
            return self.check_progressive_override(label, ctx, false);
        }
        if self.overridden_by.is_some() {
            return false;
        }
        if let Some(label) = &self.override_label {
            return self.check_progressive_override(label, ctx, true);
        }
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

#[derive(Clone)]
pub enum Edge<'a> {
    SubgraphEntrypoint {
        field_names: Vec<&'a str>,
        name: SubgraphName<'a>,
    },
    FieldMove(Box<FieldMove<'a>>),
    EntityMove(EntityMove<'a>),
    AbstractMove(&'a str),
    Selfie(&'a str),
    InterfaceObjectTypeMove(InterfaceObjectTypeMove<'a>),
}

pub type EdgeReference<'a> = GraphEdgeReference<'a, Edge<'a>>;

impl<'a> Edge<'a> {
    pub fn create_entity_move(
        key: &'a str,
        selection: std::sync::Arc<TypeAwareSelection<'a>>,
        is_interface: bool,
    ) -> Self {
        Self::EntityMove(EntityMove {
            key,
            requirements: selection,
            is_interface,
        })
    }

    pub fn create_interface_object_type_move(
        object_type_name: &'a str,
        selection: std::sync::Arc<TypeAwareSelection<'a>>,
    ) -> Self {
        Self::InterfaceObjectTypeMove(InterfaceObjectTypeMove {
            object_type_name,
            requirements: selection,
        })
    }

    pub fn create_field_move(
        name: &'a str,
        type_name: &'a str,
        is_leaf: bool,
        is_list: bool,
        join_field: Option<JoinFieldDirective>,
        requirements: Option<std::sync::Arc<TypeAwareSelection<'a>>>,
        overridden_by: Option<(String, Option<OverrideLabel>)>,
    ) -> Self {
        let override_from = join_field.as_ref().and_then(|jf| jf.override_value.clone());
        let override_label = join_field.as_ref().and_then(|jf| jf.override_label.clone());

        Self::FieldMove(Box::new(FieldMove {
            name,
            type_name,
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
            Self::Selfie(_) => "selfie",
            Self::SubgraphEntrypoint { name, .. } => &name.0,
            Self::InterfaceObjectTypeMove(InterfaceObjectTypeMove {
                object_type_name, ..
            }) => object_type_name,
        }
    }

    pub fn requirements(&self) -> Option<&TypeAwareSelection<'_>> {
        match self {
            Self::EntityMove(entity_move) => Some(entity_move.requirements.as_ref()),
            Self::InterfaceObjectTypeMove(m) => Some(m.requirements.as_ref()),
            Self::FieldMove(field_move) => field_move.requirements.as_deref(),
            _ => None,
        }
    }

    pub fn cost(&self) -> u64 {
        let move_cost = match self {
            Self::FieldMove(_) => 1,
            _ => 1000,
        };
        let requirement_cost = match self.requirements() {
            Some(selection) => selection.selection_set.cost(),
            None => 0,
        };
        move_cost + requirement_cost
    }
}

impl Display for Edge<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Edge::SubgraphEntrypoint { name, .. } => write!(f, "{}", name.0),
            Edge::EntityMove(EntityMove { .. }) => write!(f, "🔑"),
            Edge::AbstractMove(_) => write!(f, "🔮"),
            Edge::Selfie(_) => write!(f, "🤳"),
            Edge::FieldMove(field_move) => write!(f, "{}", field_move.name),
            Edge::InterfaceObjectTypeMove(m) => write!(f, "🔎 {}", m.object_type_name),
        }?;
        if let Some(reqs) = self.requirements() {
            write!(f, "🧩{}", reqs.selection_set)?
        };
        Ok(())
    }
}

impl Debug for Edge<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Edge::SubgraphEntrypoint { name, .. } => write!(f, "subgraph({})", name.0),
            Edge::FieldMove(fm) => {
                let mut result = write!(f, "{}", &fm.name);
                if let Some(jf) = &fm.join_field {
                    if let Some(req) = &jf.requires {
                        result = result.and_then(|_| write!(f, " @requires({})", req));
                    }
                    if jf.provides.is_some() {
                        result = result.and_then(|_| write!(f, " @provides"));
                    }
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
            Edge::Selfie(_) => write!(f, "🤳"),
            Edge::InterfaceObjectTypeMove(m) => write!(f, "🔎 {}", m.object_type_name),
        }
    }
}

impl PartialEq for Edge<'_> {
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

impl Eq for Edge<'_> {}
