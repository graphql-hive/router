use super::{FieldRecord, Guard, ProjectionPlan, Range};
use crate::executor::operation_filter::PathSegment;
use crate::pipeline::trie::{PathIndex, Trie};
use std::sync::Arc;

impl ProjectionPlan {
    pub(crate) fn rewrite(self: &Arc<Self>, trie: &Trie) -> Arc<Self> {
        let mut patched = None;
        rewrite_range(self, &mut patched, self.roots, trie, PathIndex::root());

        match patched {
            Some(fields) => Arc::new(ProjectionPlan {
                roots: self.roots,
                fields: fields.into_boxed_slice(),
                tables: self.tables.clone(),
            }),
            None => Arc::clone(self),
        }
    }
}

fn rewrite_range(
    source: &ProjectionPlan,
    patched: &mut Option<Vec<FieldRecord>>,
    range: Range,
    trie: &Trie,
    path: PathIndex,
) {
    if !trie.has_children(path) {
        return;
    }

    for index in range.start as usize..range.end() {
        let base = source.fields[index];
        let typed_path = match base.parent_guard.and_then(|guard| {
            let record = source.guard(guard);
            match record {
                Guard::Exact(symbol) => Some(source.text(symbol)),
                Guard::Set(_) => None,
            }
        }) {
            Some(type_name) => {
                match trie.find_segment_at_position(path, PathSegment::Fragment(type_name)) {
                    Some((position, _)) => position,
                    None => continue,
                }
            }
            None => path,
        };

        let Some((child_path, marked)) = trie
            .find_segment_at_position(typed_path, PathSegment::Field(source.response_key(&base)))
        else {
            continue;
        };

        if marked {
            let out = patched.get_or_insert_with(|| source.fields.to_vec());
            out[index].set_null();
            continue;
        }

        if base.has_children() {
            if base.children.len == 0 {
                let out = patched.get_or_insert_with(|| source.fields.to_vec());
                out[index].set_passthrough();
            } else {
                rewrite_range(source, patched, base.children, trie, child_path);
            }
        }
    }
}
