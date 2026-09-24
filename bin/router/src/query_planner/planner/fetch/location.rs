use std::{
    collections::HashMap,
    fmt::{Debug, Display},
    hash::{Hash, Hasher},
    rc::Rc,
};

use crate::query_planner::ast::merge_path::{MergePath, Segment};

/// A place in the response, like `products.@.reviews`. Each place exists once per fetch graph,
/// so comparing two of them is comparing ids, and the parent and the same place without type
/// conditions are one step away.
///
/// Only compare locations from the same `Locations`. Ids of two graphs mean nothing to each other.
#[derive(Clone)]
pub struct Location(Rc<LocationNode>);

struct LocationNode {
    id: u32,
    parent: Option<Location>,
    path: MergePath,
    /// The same place with type conditions left out. `None` when there are none, so it's itself.
    untyped: Option<Location>,
}

impl Location {
    pub fn path(&self) -> &MergePath {
        &self.0.path
    }

    pub fn is_root(&self) -> bool {
        self.0.parent.is_none()
    }

    /// The same place with every type condition left out: `a.@|[Book].b` gives `a.@.b`.
    pub fn untyped(&self) -> &Location {
        self.0.untyped.as_ref().unwrap_or(self)
    }

    /// Is this `prefix`, or somewhere below it?
    pub fn is_within(&self, prefix: &Location) -> bool {
        let depth = prefix.path().len();
        let mut current = self;
        while current.path().len() > depth {
            match &current.0.parent {
                Some(parent) => current = parent,
                None => return false,
            }
        }
        current == prefix
    }

    /// The rest of the path below `prefix`, when this is `prefix` or below it.
    pub fn strip_prefix(&self, prefix: &Location) -> Option<MergePath> {
        self.is_within(prefix)
            .then(|| self.path().slice_from(prefix.path().len()))
    }
}

impl PartialEq for Location {
    fn eq(&self, other: &Self) -> bool {
        self.0.id == other.0.id
    }
}

impl Eq for Location {}

impl Hash for Location {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.id.hash(state);
    }
}

impl Debug for Location {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Location({})", self.path())
    }
}

impl Display for Location {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.path())
    }
}

/// Hands out the `Location`s of one fetch graph.
#[derive(Clone)]
pub struct Locations {
    root: Location,
    children: HashMap<(u32, Segment), Location>,
}

impl Default for Locations {
    fn default() -> Self {
        Self {
            root: Location(Rc::new(LocationNode {
                id: 0,
                parent: None,
                path: MergePath::default(),
                untyped: None,
            })),
            children: HashMap::new(),
        }
    }
}

impl Debug for Locations {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Locations({})", self.children.len() + 1)
    }
}

impl Locations {
    pub fn root(&self) -> Location {
        self.root.clone()
    }

    pub fn get(&mut self, path: &MergePath) -> Location {
        path.inner
            .iter()
            .fold(self.root(), |parent, segment| self.child(&parent, segment))
    }

    fn child(&mut self, parent: &Location, segment: &Segment) -> Location {
        let key = (parent.0.id, segment.clone());
        if let Some(existing) = self.children.get(&key) {
            return existing.clone();
        }

        let untyped = match segment {
            Segment::TypeCondition(..) => Some(parent.untyped().clone()),
            _ if parent.0.untyped.is_none() => None,
            _ => {
                let untyped_parent = parent.untyped().clone();
                Some(self.child(&untyped_parent, segment))
            }
        };
        let location = Location(Rc::new(LocationNode {
            id: self.children.len() as u32 + 1,
            parent: Some(parent.clone()),
            path: parent.path().push(segment.clone()),
            untyped,
        }));
        self.children.insert(key, location.clone());
        location
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use crate::query_planner::ast::merge_path::{FieldPathSegment, MergePath, Segment};

    use super::Locations;

    fn path(text: &str) -> MergePath {
        MergePath::new(
            text.split('.')
                .filter(|s| !s.is_empty())
                .map(|s| match s {
                    "@" => Segment::List,
                    s if s.starts_with('|') => Segment::TypeCondition(
                        BTreeSet::from([s.trim_start_matches('|').to_string()]),
                        None,
                    ),
                    s => Segment::Field(FieldPathSegment::named(s.to_string()), 0, None),
                })
                .collect(),
        )
    }

    #[test]
    fn same_path_is_same_location() {
        let mut locations = Locations::default();
        let a = locations.get(&path("a.@.b"));
        assert_eq!(a, locations.get(&path("a.@.b")));
        assert_ne!(a, locations.get(&path("a.@.c")));
        assert_eq!(a.path(), &path("a.@.b"));
        assert!(locations.get(&path("")).is_root());
    }

    #[test]
    fn untyped_leaves_out_type_conditions() {
        let mut locations = Locations::default();
        let typed = locations.get(&path("a.@.|Book.b"));
        let other = locations.get(&path("a.@.|Movie.b"));
        assert_eq!(typed.untyped(), &locations.get(&path("a.@.b")));
        assert_eq!(typed.untyped(), other.untyped());
        let plain = locations.get(&path("a.@.b"));
        assert_eq!(plain.untyped(), &plain);
    }

    #[test]
    fn strip_prefix_is_exact() {
        let mut locations = Locations::default();
        let mut strip = |a: &str, b: &str| {
            let (a, b) = (locations.get(&path(a)), locations.get(&path(b)));
            a.strip_prefix(&b).map(|rest| rest.to_string())
        };
        assert_eq!(strip("a.@.b", "a.@"), Some("b".to_string()));
        assert_eq!(strip("a.@", "a.@"), Some("".to_string()));
        assert_eq!(strip("a.@.|Cat.b", "a.@"), Some("|[Cat].b".to_string()));
        assert_eq!(strip("a.@.b", "a.@.|Cat"), None);
        assert_eq!(strip("a", "a.b"), None);
        assert_eq!(strip("b.@", "a.@"), None);
    }
}
