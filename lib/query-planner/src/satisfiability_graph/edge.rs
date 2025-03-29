use std::fmt::Debug;

use crate::federation_spec::directives::JoinFieldDirective;

pub enum Edge {
    Root {
        field_name: String,
    },
    Field {
        name: String,
        join_field: Option<JoinFieldDirective>,
        requires: Option<String>,
        provides: Option<String>,
        override_from: Option<String>,
    }, // Field of an entity
    EntityReference(String), // 🔑 Reference to another entity using "@key"
    InterfaceImplementation(String), // 🔮 Interface implementation
}

impl Edge {
    /// Helper to create a Field edge from a field name and join directive
    pub fn field(name: String, join_field: Option<JoinFieldDirective>) -> Self {
        let requires = join_field.as_ref().and_then(|jf| jf.requires.clone());
        let provides = join_field.as_ref().and_then(|jf| jf.provides.clone());
        let override_from = join_field.as_ref().and_then(|jf| jf.override_value.clone());

        Self::Field {
            name,
            join_field,
            requires,
            provides,
            override_from,
        }
    }

    pub fn is_entity_reference(&self) -> bool {
        matches!(self, Self::EntityReference(_))
    }

    pub fn id(&self) -> &str {
        match self {
            Self::Field { name, .. } => name,
            Self::EntityReference(id) => id,
            Self::InterfaceImplementation(id) => id,
            Self::Root { field_name } => field_name,
        }
    }

    /// Returns true if this edge can provide the specified field
    pub fn provides_field(&self, field_name: &str) -> bool {
        match self {
            Self::Field {
                provides: Some(provides),
                ..
            } => {
                // Check if the provides directive includes this field
                // This is a simplified check - in a full implementation you'd want to parse
                // the provides string and check if it contains the field
                provides.contains(field_name)
            }
            _ => false,
        }
    }

    /// Returns true if this edge requires other fields
    pub fn has_requirements(&self) -> bool {
        match self {
            Self::Field { requires, .. } => requires.is_some(),
            _ => false,
        }
    }

    /// Gets the requirements as a string, if any
    pub fn get_requirements(&self) -> Option<&String> {
        match self {
            Self::Field { requires, .. } => requires.as_ref(),
            _ => None,
        }
    }
}

impl Debug for Edge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Edge::Root { field_name } => write!(f, "root({})", field_name),

            Edge::Field {
                name, join_field, ..
            } => {
                // Start with the field name
                let mut result = write!(f, "{}", name);

                // Add requires directive if present
                if let Some(jf) = join_field {
                    if let Some(req) = &jf.requires {
                        result = result.and_then(|_| write!(f, " @requires({})", req));
                    }

                    // Add provides directive if present
                    if let Some(prov) = &jf.provides {
                        result = result.and_then(|_| write!(f, " @provides({})", prov));
                    }

                    // Add other relevant directives like external, override, etc.
                    if jf.external.unwrap_or(false) {
                        result = result.and_then(|_| write!(f, " @external"));
                    }

                    if let Some(override_from) = &jf.override_value {
                        result =
                            result.and_then(|_| write!(f, " @override(from: {})", override_from));
                    }
                }

                result
            }

            Edge::EntityReference(name) => write!(f, "🔑 {}", name),
            Edge::InterfaceImplementation(name) => write!(f, "🔮 {}", name),
        }
    }
}
impl PartialEq for Edge {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Edge::Root { field_name },
                Edge::Root {
                    field_name: other_field_name,
                },
            ) => field_name == other_field_name,
            (
                Edge::Field {
                    name,
                    join_field: Some(jf1),
                    ..
                },
                Edge::Field {
                    name: other_name,
                    join_field: Some(jf2),
                    ..
                },
            ) => {
                // Compare names and directive fields that affect planning
                name == other_name
                    && jf1.requires == jf2.requires
                    && jf1.provides == jf2.provides
                    && jf1.external == jf2.external
                    && jf1.override_value == jf2.override_value
            }

            (
                Edge::Field {
                    name,
                    join_field: None,
                    ..
                },
                Edge::Field {
                    name: other_name,
                    join_field: None,
                    ..
                },
            ) => name == other_name,

            (Edge::EntityReference(name), Edge::EntityReference(other_name)) => name == other_name,

            (Edge::InterfaceImplementation(name), Edge::InterfaceImplementation(other_name)) => {
                name == other_name
            }

            _ => false,
        }
    }
}
