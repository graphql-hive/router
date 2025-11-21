use crate::pipeline::authorization::collector::{
    CheckIndex, FieldAuthStatus, FieldCheck, PathSegment,
};
use crate::utils::StrByAddr;
use ahash::HashMap;

/// Type-safe position in the path trie structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct PathIndex(usize);

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
    pub(super) fn root() -> Self {
        Self(0)
    }
}

/// Node in the path trie for tracking unauthorized field paths.
#[derive(Debug, Default)]
pub(super) struct PathNode<'op> {
    /// Mapping from field name/alias to child position in UnauthorizedPathTrie.nodes
    child_fields: HashMap<StrByAddr<'op>, PathIndex>,
    /// If true, the field corresponding to this path is unauthorized.
    is_unauthorized: bool,
}

/// An index-based (flattened) trie for storing unauthorized field paths efficiently.
///
/// For unauthorized paths like `["user", "posts", "title"]` and `["user", "email"]`:
///
/// ```text
/// nodes[0] (root)
///   └─ "user" -> nodes[1]
///       ├─ "posts" -> nodes[2]
///       │   └─ "title" -> nodes[3] [UNAUTHORIZED]
///       └─ "email" -> nodes[4] [UNAUTHORIZED]
/// ```
///
/// The flattened representation:
/// - 0 - root with child "user" -> 1
/// - 1 - "user" with children {"posts" -> 2, "email" -> 4}
/// - 2 - "posts" with child "title" -> 3
/// - 3 - "title" marked as unauthorized
/// - 4 - "email" marked as unauthorized
#[derive(Debug)]
pub(super) struct UnauthorizedPathTrie<'op> {
    /// The root is always at position 0.
    nodes: Vec<PathNode<'op>>,
}

impl<'op> UnauthorizedPathTrie<'op> {
    /// Creates a new lookup with an empty root entry.
    fn new() -> Self {
        Self {
            nodes: vec![PathNode::default()], // Root entry at position 0
        }
    }

    /// Builds trie of unauthorized paths for cheap lookups during reconstruction.
    pub(super) fn from_checks(
        checks: &[FieldCheck<'op>],
        removal_flags: &[bool],
    ) -> UnauthorizedPathTrie<'op> {
        let mut unauthorized_path_trie = UnauthorizedPathTrie::new();
        let mut path_buffer = Vec::with_capacity(16);

        for (i, check) in checks.iter().enumerate() {
            let should_remove =
                removal_flags[i] || check.status == FieldAuthStatus::UnauthorizedNullable;

            if !should_remove {
                continue;
            }

            let mut current_check_index = Some(CheckIndex::new(i));
            while let Some(index) = current_check_index {
                let check = &checks[index.get()];
                path_buffer.push(check.path_segment);
                current_check_index = check.parent_check_index;
            }

            path_buffer.reverse();
            unauthorized_path_trie.add_unauthorized_path(&path_buffer);
            path_buffer.clear();
        }
        unauthorized_path_trie
    }

    /// Records a path to an unauthorized field.
    ///
    /// Builds the trie structure by following the path segments and creating
    /// entries as needed. The final segment is marked as unauthorized.
    fn add_unauthorized_path(&mut self, path: &[PathSegment<'op>]) {
        let mut current_path_position = PathIndex::root();
        for segment in path {
            let segment_key = StrByAddr(segment.as_str());

            if let Some(&child_path_position) = self.nodes[current_path_position.get()]
                .child_fields
                .get(&segment_key)
            {
                current_path_position = child_path_position;
            } else {
                let new_path_position = PathIndex::new(self.nodes.len());
                self.nodes.push(PathNode::default());

                let parent_node = &mut self.nodes[current_path_position.get()];
                parent_node
                    .child_fields
                    .insert(segment_key, new_path_position);

                current_path_position = new_path_position;
            }
        }

        self.nodes[current_path_position.get()].is_unauthorized = true;
    }

    /// Finds the child entry for a given field name at the specified position.
    #[inline]
    pub(super) fn find_field(
        &self,
        parent_path_position: PathIndex,
        segment: &'op str,
    ) -> Option<(PathIndex, bool)> {
        let parent_node = &self.nodes[parent_path_position.get()];
        let child_path_position = parent_node.child_fields.get(&StrByAddr(segment)).copied()?;
        let child_node = &self.nodes[child_path_position.get()];
        Some((child_path_position, child_node.is_unauthorized))
    }

    /// Returns true if any unauthorized fields exist in this subtree.
    #[inline]
    pub(super) fn has_unauthorized_fields(&self, path_position: PathIndex) -> bool {
        !self.nodes[path_position.get()].child_fields.is_empty()
    }
}
