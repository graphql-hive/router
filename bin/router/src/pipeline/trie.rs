use ahash::HashMap;
use lasso2::{Capacity, Rodeo, Spur};

/// Type-safe position in the path trie structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct PathIndex(usize);

impl PathIndex {
    #[inline]
    fn new(index: usize) -> Self {
        Self(index)
    }

    #[inline]
    fn get(self) -> usize {
        self.0
    }

    /// Returns the root position (always 0)
    #[inline]
    pub(crate) fn root() -> Self {
        Self(0)
    }
}

/// Node in the path trie for tracking "marked" field paths.
#[derive(Debug, Default)]
struct PathNode {
    /// Mapping from interned field name to child position.
    children: HashMap<Spur, PathIndex>,
    /// If true, the field corresponding to this path is marked. What a mark
    /// means is entirely up to the caller (e.g. nulling, per `rebuilder.rs`).
    marked: bool,
}

/// An index-based (flattened) trie for marking field paths efficiently and
/// querying them during a selection-set walk. Field names are interned to
/// allocate less memory than storing raw strings. This structure carries no
/// opinion about what a "mark" means - see its consumers (e.g.
/// `nullify::rebuilder`) for that.
///
/// For marked paths like `["user", "posts", "title"]` and `["user", "email"]`:
///
/// ```text
/// nodes[0] (root)
///   └─ "user" -> nodes[1]
///       ├─ "posts" -> nodes[2]
///       │   └─ "title" -> nodes[3] [MARKED]
///       └─ "email" -> nodes[4] [MARKED]
/// ```
///
/// The flattened representation:
/// - 0 - root with child "user" -> 1
/// - 1 - "user" with children {"posts" -> 2, "email" -> 4}
/// - 2 - "posts" with child "title" -> 3
/// - 3 - "title" marked
/// - 4 - "email" marked
#[derive(Debug)]
pub(crate) struct Trie {
    /// The root is always at position 0.
    nodes: Vec<PathNode>,
    interner: Rodeo,
}

impl Trie {
    /// Creates a new lookup with an empty root entry, pre-sized to
    /// accommodate `node_count` nodes and `segment_count` interned segments.
    fn with_capacity(node_count: usize, segment_count: usize) -> Self {
        let mut nodes = Vec::with_capacity(node_count + 1);
        nodes.push(PathNode::default()); // Root entry at position 0
        Self {
            nodes,
            interner: Rodeo::with_capacity(Capacity::for_strings(segment_count)),
        }
    }

    /// Builds a trie from a list of paths, marking each one.
    pub(crate) fn from_paths(paths: &[Vec<&str>]) -> Self {
        let segment_count: usize = paths.iter().map(|path| path.len()).sum();
        let mut trie = Self::with_capacity(segment_count, segment_count);
        for path in paths {
            trie.add_path(path.iter().copied());
        }
        trie
    }

    pub(crate) fn add_path(&mut self, segments: impl Iterator<Item = impl AsRef<str>>) {
        let mut current = PathIndex::root();
        let mut peekable = segments.peekable();

        loop {
            let Some(segment) = peekable.next() else {
                break;
            };
            let is_last = peekable.peek().is_none();
            let spur = self.interner.get_or_intern(segment.as_ref());
            let child = self.nodes[current.get()].children.get(&spur).copied();

            let next = match child {
                Some(child_idx) => child_idx,
                None => {
                    let new_idx = PathIndex::new(self.nodes.len());
                    self.nodes.push(PathNode::default());
                    self.nodes[current.get()].children.insert(spur, new_idx);
                    new_idx
                }
            };

            current = next;

            if is_last {
                self.nodes[current.get()].marked = true;
            }
        }
    }

    /// Finds the child entry for a segment at the specified position.
    /// Returns `(child_position, is_marked)` if the segment is known,
    /// or `None` if it's not in the trie.
    #[inline]
    pub(super) fn find_segment_at_position(
        &self,
        parent_path_position: PathIndex,
        segment: &str,
    ) -> Option<(PathIndex, bool)> {
        let spur = self.interner.get(segment)?;
        let parent_node = &self.nodes[parent_path_position.get()];
        let child_path_position = parent_node.children.get(&spur).copied()?;
        let child_node = &self.nodes[child_path_position.get()];
        Some((child_path_position, child_node.marked))
    }

    #[inline]
    pub(super) fn has_children(&self, path_position: PathIndex) -> bool {
        !self.nodes[path_position.get()].children.is_empty()
    }
}
