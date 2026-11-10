//! Normalizes an operation, or a federation field set, on a flat `Document`.
//!
//! The passes run in order over the whole operation: drop skipped selections, tidy fields,
//! inline fragment spreads, flatten fragments into object types, expand interface fields,
//! merge inline fragments, flatten object fragments, merge fields, drop repeated fields.
//!
//! The first three run before any subtree is shared, so they change nodes in place. From
//! inlining fragments on, nodes are only appended: a new list for a changed selection set, a
//! new node for a changed selection, the rest is reused by index.

use ahash::{HashMap, HashMapExt, HashSet};

use super::{
    Definition, Document, Fragment, Interner, Kind, Operation, OperationKind, SelId, Selection,
    Span, Sym, Value, ValueId, NONE,
};
use crate::query_planner::ast::document::NormalizedDocument;
use crate::query_planner::ast::normalization::error::NormalizationError;
use crate::query_planner::ast::selection_set::SelectionSet;
use crate::query_planner::state::supergraph_state::{
    SupergraphDefinition, SupergraphField, SupergraphObjectType, SupergraphState,
};

/// A field of a schema type, and the type of its value.
#[derive(Clone, Copy)]
pub(super) struct FieldInfo<'a> {
    pub(super) field: &'a SupergraphField,
    pub(super) ty: Option<&'a SupergraphDefinition>,
}

pub(super) struct Norm<'a> {
    pub(super) state: &'a SupergraphState,
    /// Possible types only in this subgraph, for a field set (`@key`, `@requires`, ...).
    subgraph: Option<&'a str>,
    /// The type a field set is on, instead of the query type.
    query_root: Option<&'a str>,
    pub(super) doc: &'a Document,
    /// Names the document doesn't have, the type names fragments get from the schema.
    pub(super) extra: Interner,
    pub(super) args: Vec<super::Argument>,
    pub(super) directives: Vec<super::Directive>,
    pub(super) sels: Vec<Selection>,
    pub(super) kids: Vec<SelId>,
    /// Lists being built. A nested call builds on top and takes its items off when it's done.
    scratch: Vec<SelId>,
    skip: Option<Sym>,
    include: Option<Sym>,
    if_: Option<Sym>,
    typename: Option<Sym>,
    /// The schema type by that name, for every name. Looked up once, a name is an index here.
    pub(super) defs: Vec<Option<&'a SupergraphDefinition>>,
    /// The object types of the abstract type by that name, for every name.
    possible: Vec<Option<&'a HashSet<String>>>,
    /// A type's field by name, keyed by the type's fields map and the name.
    fields: HashMap<(usize, Sym), Option<FieldInfo<'a>>>,
    /// Whether a field of an interface goes out as a fragment per object, see `expand_field`.
    expand: HashMap<(usize, Sym), bool>,
    /// The objects of an interface, sorted by name.
    interface_objects: HashMap<usize, std::rc::Rc<[&'a SupergraphObjectType]>>,
}

type Result<T> = std::result::Result<T, NormalizationError>;

pub fn normalize_operation(
    state: &SupergraphState,
    doc: &Document,
    operation_name: Option<&str>,
) -> Result<NormalizedDocument> {
    let (mut n, op) = run(state, doc, operation_name, None, None)?;
    Ok(super::convert::normalized_document(
        &mut n,
        op,
        operation_name,
    ))
}

/// A field set of a federation directive, like `@key(fields: "id")` or `@requires`, on
/// `type_name` in a subgraph. Normalized like `{ <fields> }` with `type_name` as the root,
/// the possible types of abstract types limited to that subgraph.
pub fn normalize_field_set(
    state: &SupergraphState,
    type_name: &str,
    subgraph_name: &str,
    fields: &str,
) -> std::result::Result<SelectionSet, String> {
    let doc = super::parse::parse(&format!("{{{fields}}}")).map_err(|err| err.to_string())?;
    let (mut n, op) = run(state, &doc, None, Some(type_name), Some(subgraph_name))
        .map_err(|err| err.to_string())?;
    // no `__typename` step, that's for the operations the router executes
    Ok(super::convert::selection_set(
        &mut n,
        op.selection_set,
        None,
    ))
}

fn run<'a>(
    state: &'a SupergraphState,
    doc: &'a Document,
    operation_name: Option<&str>,
    query_root: Option<&'a str>,
    subgraph: Option<&'a str>,
) -> Result<(Norm<'a>, Operation)> {
    let mut n = Norm {
        state,
        subgraph,
        query_root,
        doc,
        extra: Interner::default(),
        args: doc.args.clone(),
        directives: doc.directives.clone(),
        sels: doc.selections.clone(),
        kids: doc.children.clone(),
        scratch: Vec::new(),
        skip: doc.strings.get("skip"),
        include: doc.strings.get("include"),
        if_: doc.strings.get("if"),
        typename: doc.strings.get("__typename"),
        defs: Vec::with_capacity(doc.strings.len()),
        possible: Vec::with_capacity(doc.strings.len()),
        fields: HashMap::new(),
        expand: HashMap::new(),
        interface_objects: HashMap::new(),
    };
    for sym in 0..doc.strings.len() {
        n.add_name(doc.strings.str(sym as Sym));
    }

    // drop_unused_operations
    let mut chosen = None;
    for definition in &doc.definitions {
        if let Definition::Operation(op) = definition {
            if chosen.is_some() {
                continue;
            }
            let matches = match operation_name {
                None => true,
                Some(name) => op.name != NONE && doc.str(op.name) == name,
            };
            if matches {
                chosen = Some(*op);
            }
        }
    }
    let Some(mut op) = chosen else {
        return Err(match operation_name {
            Some(name) => NormalizationError::SpecifiedOperationNotFound {
                operation_name: name.to_string(),
            },
            None => NormalizationError::OperationNotFound,
        });
    };
    // the operation and the fragments, in document order
    let mut fragments: Vec<Fragment> = Vec::new();
    let mut op_position = 0;
    for definition in &doc.definitions {
        match definition {
            Definition::Operation(o) if o.selection_set == op.selection_set => {
                op_position = fragments.len()
            }
            Definition::Fragment(f) => fragments.push(*f),
            _ => {}
        }
    }

    // drop_skipped_fields
    op.selection_set = n.drop_skipped(op.selection_set);
    for fragment in &mut fragments {
        fragment.selection_set = n.drop_skipped(fragment.selection_set);
    }

    // normalize_fields
    n.normalize_fields(op.selection_set);
    for fragment in &fragments {
        n.normalize_fields(fragment.selection_set);
    }

    // inline_fragment_spreads, the fragments only for their errors
    let mut fragment_map: HashMap<Sym, Fragment> = HashMap::with_capacity(fragments.len());
    for fragment in &fragments {
        fragment_map.insert(fragment.name, *fragment);
    }
    for (index, fragment) in fragments.iter().enumerate() {
        if index == op_position {
            op.selection_set =
                n.inline_spreads(op.selection_set, NONE, &fragment_map, &mut Vec::new())?;
        }
        n.inline_spreads(
            fragment.selection_set,
            fragment.type_condition,
            &fragment_map,
            &mut Vec::new(),
        )?;
    }
    if op_position == fragments.len() {
        op.selection_set =
            n.inline_spreads(op.selection_set, NONE, &fragment_map, &mut Vec::new())?;
    }
    drop(fragment_map);

    let root = n.root_type(op.kind)?;
    op.selection_set = n.flatten_fragments(op.selection_set, root)?;
    op.selection_set = n.type_expand(op.selection_set, root)?;
    op.selection_set = n.merge_inline_fragments(op.selection_set, root)?;
    op.selection_set = n.flatten_matching_object_fragments(op.selection_set, root)?;
    op.selection_set = n.merge_fields(op.selection_set)?;
    op.selection_set = n.drop_duplicated_fields(op.selection_set);

    Ok((n, op))
}

impl<'a> Norm<'a> {
    #[inline]
    pub(super) fn s(&self, sym: Sym) -> &str {
        let base = self.doc.strings.len() as Sym;
        match sym < base {
            true => self.doc.strings.str(sym),
            false => self.extra.str(sym - base),
        }
    }

    fn intern(&mut self, s: &str) -> Sym {
        if let Some(sym) = self.doc.strings.get(s) {
            return sym;
        }
        let before = self.extra.len();
        let sym = self.doc.strings.len() as Sym + self.extra.intern(s);
        if self.extra.len() > before {
            self.add_name(s);
        }
        sym
    }

    /// Looks up what the schema has by that name, for the next name.
    fn add_name(&mut self, name: &str) {
        self.defs.push(self.state.definitions.get(name));
        let possible = match self.subgraph {
            Some(subgraph) => self.state.possible_types_in_subgraph(name, subgraph),
            None => self.state.all_possible_types(name),
        };
        self.possible.push(possible);
    }

    pub(super) fn field_info(
        &mut self,
        fields: &'a HashMap<String, SupergraphField>,
        name: Sym,
    ) -> Option<FieldInfo<'a>> {
        let key = (fields as *const _ as usize, name);
        if let Some(info) = self.fields.get(&key) {
            return *info;
        }
        let info = fields.get(self.s(name)).map(|field| FieldInfo {
            field,
            ty: self.state.definitions.get(field.field_type.inner_type()),
        });
        self.fields.insert(key, info);
        info
    }

    fn definition_of(&self, sym: Sym) -> Result<&'a SupergraphDefinition> {
        self.defs[sym as usize].ok_or_else(|| NormalizationError::SchemaTypeNotFound {
            type_name: self.s(sym).to_string(),
        })
    }

    #[inline]
    fn sel(&self, list: Span, index: usize) -> (SelId, Selection) {
        let id = self.kids[list.start as usize + index];
        (id, self.sels[id as usize])
    }

    /// The list built on the scratch stack since `mark`, as `original` when it's the same.
    fn finish(&mut self, mark: usize, original: Span) -> Span {
        if self.scratch[mark..] == self.kids[original.range()] {
            self.scratch.truncate(mark);
            return original;
        }
        self.list(mark)
    }

    fn list(&mut self, mark: usize) -> Span {
        let start = self.kids.len() as u32;
        self.kids.extend(self.scratch.drain(mark..));
        Span {
            start,
            len: self.kids.len() as u32 - start,
        }
    }

    fn push_sel(&mut self, selection: Selection) -> SelId {
        self.sels.push(selection);
        self.sels.len() as SelId - 1
    }

    fn with_children(&mut self, id: SelId, children: Span) -> SelId {
        let selection = self.sels[id as usize];
        if selection.children == children {
            return id;
        }
        self.push_sel(Selection {
            children,
            ..selection
        })
    }

    fn extend_scratch(&mut self, list: Span) {
        self.scratch.extend_from_slice(&self.kids[list.range()]);
    }

    fn concat_directives(&mut self, a: Span, b: Span) -> Span {
        if b.is_empty() {
            return a;
        }
        if a.is_empty() {
            return b;
        }
        let start = self.directives.len() as u32;
        self.directives.extend_from_within(a.range());
        self.directives.extend_from_within(b.range());
        Span {
            start,
            len: a.len + b.len,
        }
    }

    fn value_eq(&self, a: ValueId, b: ValueId) -> bool {
        if a == b {
            return true;
        }
        let doc = self.doc;
        match (doc.values[a as usize], doc.values[b as usize]) {
            (Value::Variable(a), Value::Variable(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Boolean(a), Value::Boolean(b)) => a == b,
            (Value::Null, Value::Null) => true,
            (Value::Enum(a), Value::Enum(b)) => a == b,
            (Value::List(a), Value::List(b)) => {
                a.len == b.len
                    && doc.value_items[a.range()]
                        .iter()
                        .zip(&doc.value_items[b.range()])
                        .all(|(a, b)| self.value_eq(*a, *b))
            }
            (Value::Object(a), Value::Object(b)) => {
                a.len == b.len
                    && doc.object_fields[a.range()]
                        .iter()
                        .zip(&doc.object_fields[b.range()])
                        .all(|(a, b)| a.0 == b.0 && self.value_eq(a.1, b.1))
            }
            _ => false,
        }
    }

    fn args_eq(&self, a: Span, b: Span) -> bool {
        a == b
            || (a.len == b.len
                && self.args[a.range()]
                    .iter()
                    .zip(&self.args[b.range()])
                    .all(|(a, b)| a.name == b.name && self.value_eq(a.value, b.value)))
    }

    fn directives_eq(&self, a: Span, b: Span) -> bool {
        a == b
            || (a.len == b.len
                && self.directives[a.range()]
                    .iter()
                    .zip(&self.directives[b.range()])
                    .all(|(a, b)| a.name == b.name && self.args_eq(a.args, b.args)))
    }

    fn root_type(&self, kind: OperationKind) -> Result<&'a SupergraphDefinition> {
        let state = self.state;
        let name = match kind {
            OperationKind::Query | OperationKind::Shorthand => {
                self.query_root.unwrap_or(state.query_type.as_str())
            }
            OperationKind::Mutation => state.mutation_type.as_deref().ok_or_else(|| {
                NormalizationError::TypeForOperationNotFound {
                    kind: "mutation".to_string(),
                }
            })?,
            OperationKind::Subscription => state.subscription_type.as_deref().ok_or_else(|| {
                NormalizationError::TypeForOperationNotFound {
                    kind: "subscription".to_string(),
                }
            })?,
        };
        self.definition(name)
    }

    fn definition(&self, name: &str) -> Result<&'a SupergraphDefinition> {
        self.state
            .definitions
            .get(name)
            .ok_or_else(|| NormalizationError::SchemaTypeNotFound {
                type_name: name.to_string(),
            })
    }

    /// The type of the field's value, for a field with a selection set.
    fn field_type(
        &mut self,
        parent: &'a SupergraphDefinition,
        field: Sym,
    ) -> Result<&'a SupergraphDefinition> {
        let info = self.field_info(parent.fields(), field).ok_or_else(|| {
            NormalizationError::FieldNotFoundInType {
                field_name: self.s(field).to_string(),
                type_name: parent.name().to_string(),
            }
        })?;
        info.ty
            .ok_or_else(|| NormalizationError::SchemaTypeNotFound {
                type_name: info.field.field_type.inner_type().to_string(),
            })
    }

    // ---- drop_skipped_fields

    fn condition(&self, directives: Span, name: Option<Sym>) -> Option<bool> {
        let name = name?;
        self.directives[directives.range()]
            .iter()
            .filter(|directive| directive.name == name)
            .find_map(|directive| {
                self.args[directive.args.range()].iter().find_map(|arg| {
                    if Some(arg.name) != self.if_ {
                        return None;
                    }
                    match self.doc.values[arg.value as usize] {
                        Value::Boolean(b) => Some(b),
                        _ => None,
                    }
                })
            })
    }

    fn should_keep(&self, directives: Span) -> bool {
        if directives.is_empty() {
            return true;
        }
        if self.condition(directives, self.skip) == Some(true) {
            return false;
        }
        self.condition(directives, self.include) != Some(false)
    }

    fn drop_skipped(&mut self, list: Span) -> Span {
        let mut write = list.start as usize;
        for index in list.range() {
            let id = self.kids[index];
            let selection = self.sels[id as usize];
            if !self.should_keep(selection.directives) {
                continue;
            }
            if selection.kind != Kind::FragmentSpread && !selection.children.is_empty() {
                self.sels[id as usize].children = self.drop_skipped(selection.children);
            }
            self.kids[write] = id;
            write += 1;
        }
        Span {
            start: list.start,
            len: write as u32 - list.start,
        }
    }

    // ---- normalize_fields: an alias that's the field's name goes, arguments and directives
    // get sorted. Only the top-level fields and the ones in inline fragments, never inside a
    // field's selections.

    fn normalize_fields(&mut self, list: Span) {
        let doc: &'a Document = self.doc;
        let strings = &doc.strings;
        for index in list.range() {
            let id = self.kids[index] as usize;
            let selection = self.sels[id];
            match selection.kind {
                Kind::Field => {
                    if selection.alias != NONE && selection.alias == selection.name {
                        self.sels[id].alias = NONE;
                    }
                    self.args[selection.args.range()]
                        .sort_by(|a, b| strings.str(a.name).cmp(strings.str(b.name)));
                    self.directives[selection.directives.range()]
                        .sort_by(|a, b| strings.str(a.name).cmp(strings.str(b.name)));
                }
                Kind::InlineFragment => {
                    self.normalize_fields(selection.children);
                    self.directives[selection.directives.range()]
                        .sort_by(|a, b| strings.str(a.name).cmp(strings.str(b.name)));
                }
                Kind::FragmentSpread => {}
            }
        }
    }

    // ---- inline_fragment_spreads

    fn inline_spreads(
        &mut self,
        list: Span,
        parent_type_condition: Sym,
        fragments: &HashMap<Sym, Fragment>,
        active: &mut Vec<Sym>,
    ) -> Result<Span> {
        let mark = self.scratch.len();
        for index in 0..list.len as usize {
            let (id, selection) = self.sel(list, index);
            match selection.kind {
                Kind::Field => {
                    let children =
                        self.inline_spreads(selection.children, NONE, fragments, active)?;
                    let id = self.with_children(id, children);
                    self.scratch.push(id);
                }
                Kind::InlineFragment => {
                    let children = self.inline_spreads(
                        selection.children,
                        selection.type_condition,
                        fragments,
                        active,
                    )?;
                    let id = self.with_children(id, children);
                    self.scratch.push(id);
                }
                Kind::FragmentSpread => {
                    let mut spread = selection;
                    let max_chain = fragments.len();
                    let mut chain_len = 0usize;
                    let fragment = loop {
                        let fragment = fragments.get(&spread.name).ok_or_else(|| {
                            NormalizationError::FragmentDefinitionNotFound {
                                fragment_name: self.s(spread.name).to_string(),
                            }
                        })?;
                        let inlineable = parent_type_condition != NONE
                            && parent_type_condition == fragment.type_condition
                            && spread.directives.is_empty();
                        if inlineable && fragment.selection_set.len == 1 {
                            let (_, next) = self.sel(fragment.selection_set, 0);
                            if next.kind == Kind::FragmentSpread {
                                chain_len += 1;
                                if chain_len > max_chain {
                                    return Err(NormalizationError::CyclicFragmentSpread {
                                        fragment_name: self.s(spread.name).to_string(),
                                    });
                                }
                                spread = next;
                                continue;
                            }
                        }
                        break *fragment;
                    };

                    if active.contains(&fragment.name) {
                        return Err(NormalizationError::CyclicFragmentSpread {
                            fragment_name: self.s(spread.name).to_string(),
                        });
                    }
                    active.push(fragment.name);
                    if parent_type_condition != NONE
                        && parent_type_condition == fragment.type_condition
                        && spread.directives.is_empty()
                    {
                        let inlined = self.inline_spreads(
                            fragment.selection_set,
                            parent_type_condition,
                            fragments,
                            active,
                        )?;
                        self.extend_scratch(inlined);
                    } else {
                        let children = self.inline_spreads(
                            fragment.selection_set,
                            fragment.type_condition,
                            fragments,
                            active,
                        )?;
                        let id = self.push_sel(Selection {
                            kind: Kind::InlineFragment,
                            name: NONE,
                            alias: NONE,
                            type_condition: fragment.type_condition,
                            args: Span::EMPTY,
                            directives: spread.directives,
                            children,
                        });
                        self.scratch.push(id);
                    }
                    active.pop();
                }
            }
        }
        Ok(self.finish(mark, list))
    }

    // ---- flatten_fragments

    fn flatten_fragments(&mut self, list: Span, parent: &'a SupergraphDefinition) -> Result<Span> {
        let mark = self.scratch.len();
        for index in 0..list.len as usize {
            let (id, selection) = self.sel(list, index);
            match selection.kind {
                Kind::Field => {
                    if self.s(selection.name).starts_with("__") || selection.children.is_empty() {
                        self.scratch.push(id);
                        continue;
                    }
                    let inner = self.field_type(parent, selection.name)?;
                    let children = self.flatten_fragments(selection.children, inner)?;
                    let id = self.with_children(id, children);
                    self.scratch.push(id);
                }
                Kind::InlineFragment => self.flatten_inline_fragment(parent, id, selection)?,
                Kind::FragmentSpread => {}
            }
        }
        Ok(self.finish(mark, list))
    }

    fn flatten_inline_fragment(
        &mut self,
        parent: &'a SupergraphDefinition,
        id: SelId,
        fragment: Selection,
    ) -> Result<()> {
        let had_no_type_condition = fragment.type_condition == NONE;
        let matches_parent =
            had_no_type_condition || self.s(fragment.type_condition) == parent.name();
        if !matches_parent {
            return self.expand_fragment_with_type_condition(parent, id, fragment);
        }
        let children = self.flatten_fragments(fragment.children, parent)?;
        if fragment.directives.is_empty() {
            self.extend_scratch(children);
            return Ok(());
        }
        let mut fragment = Selection {
            children,
            ..fragment
        };
        if had_no_type_condition {
            fragment.type_condition = self.intern(parent.name());
            if matches!(
                parent,
                SupergraphDefinition::Interface(_) | SupergraphDefinition::Union(_)
            ) {
                return self.expand_abstract_fragment(parent, fragment);
            }
        }
        if self.push_conditions_down(fragment) {
            return Ok(());
        }
        let id = match fragment.type_condition == self.sels[id as usize].type_condition {
            true => self.with_children(id, children),
            false => self.push_sel(fragment),
        };
        self.scratch.push(id);
        Ok(())
    }

    /// `... on Animal @include(if: $x) { ... on Dog { tricks } }` on an `Animal` is
    /// `... on Dog @include(if: $x) { tricks }`. Kept as is, those Dogs are at another path than a
    /// `... on Dog` next to it, and get looked up twice. Pushes the children on the scratch list
    /// and returns true, or false when a child isn't an inline fragment or already has the same
    /// directive with other arguments, the two can't go on one fragment.
    fn push_conditions_down(&mut self, fragment: Selection) -> bool {
        let conditions = fragment.directives;
        let (skip, include) = (self.skip, self.include);
        if self.directives[conditions.range()]
            .iter()
            .any(|d| Some(d.name) != skip && Some(d.name) != include)
        {
            return false;
        }
        let rollback = self.directives.len();
        let mut children = Vec::with_capacity(fragment.children.len as usize);
        for index in 0..fragment.children.len as usize {
            let (_, child) = self.sel(fragment.children, index);
            if child.kind != Kind::InlineFragment {
                self.directives.truncate(rollback);
                return false;
            }
            let start = self.directives.len() as u32;
            self.directives.extend_from_within(child.directives.range());
            for at in conditions.range() {
                let condition = self.directives[at].clone();
                let existing = self.directives[child.directives.range()]
                    .iter()
                    .find(|d| d.name == condition.name)
                    .map(|d| d.args);
                match existing {
                    Some(args) if self.args_eq(args, condition.args) => {}
                    Some(_) => {
                        self.directives.truncate(rollback);
                        return false;
                    }
                    None => self.directives.push(condition),
                }
            }
            let directives = Span {
                start,
                len: self.directives.len() as u32 - start,
            };
            children.push(Selection { directives, ..child });
        }
        for child in children {
            let id = self.push_sel(child);
            self.scratch.push(id);
        }
        true
    }

    fn expand_fragment_with_type_condition(
        &mut self,
        parent: &'a SupergraphDefinition,
        id: SelId,
        fragment: Selection,
    ) -> Result<()> {
        let type_condition = self.definition_of(fragment.type_condition)?;
        match type_condition {
            SupergraphDefinition::Interface(_) | SupergraphDefinition::Union(_) => {
                self.expand_abstract_fragment(parent, fragment)
            }
            SupergraphDefinition::Object(_) => {
                if matches!(parent, SupergraphDefinition::Object(_))
                    && parent.name() != type_condition.name()
                {
                    return Ok(());
                }
                let children = self.flatten_fragments(fragment.children, type_condition)?;
                let id = self.with_children(id, children);
                self.scratch.push(id);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn possible_types(&self, name: &str) -> Result<&'a HashSet<String>> {
        let possible = match self.subgraph {
            Some(subgraph) => self.state.possible_types_in_subgraph(name, subgraph),
            None => self.state.all_possible_types(name),
        };
        possible.ok_or_else(|| NormalizationError::PossibleTypesNotFound {
            type_name: name.to_string(),
        })
    }

    fn fragment_applies_to_object(&self, fragment: &Selection, object: &str) -> bool {
        if fragment.type_condition == NONE {
            return true;
        }
        self.s(fragment.type_condition) == object
            || self.possible[fragment.type_condition as usize]
                .is_some_and(|types| types.contains(object))
    }

    fn expand_abstract_fragment(
        &mut self,
        parent: &'a SupergraphDefinition,
        fragment: Selection,
    ) -> Result<()> {
        let of_type_condition =
            self.possible[fragment.type_condition as usize].ok_or_else(|| {
                NormalizationError::PossibleTypesNotFound {
                    type_name: self.s(fragment.type_condition).to_string(),
                }
            })?;
        let owned_parent_set: HashSet<String>;
        let of_parent = match parent {
            SupergraphDefinition::Union(_) | SupergraphDefinition::Interface(_) => {
                self.possible_types(parent.name())?
            }
            _ => {
                owned_parent_set = HashSet::from_iter([parent.name().to_string()]);
                &owned_parent_set
            }
        };
        let mut objects: Vec<&str> = of_type_condition
            .intersection(of_parent)
            .map(String::as_str)
            .collect();
        objects.sort_unstable();

        let items = fragment.children;
        for object in objects {
            let object_def = self.definition(object)?;
            let object_sym = self.intern(object);

            // the fields, then the inline fragments that apply, both for this object
            let fields_mark = self.scratch.len();
            for index in 0..items.len as usize {
                let (id, selection) = self.sel(items, index);
                if selection.kind == Kind::Field {
                    self.scratch.push(id);
                }
            }
            let inherited = self.list(fields_mark);
            let specific_mark = self.scratch.len();
            for index in 0..items.len as usize {
                let (id, selection) = self.sel(items, index);
                if selection.kind == Kind::InlineFragment
                    && self.fragment_applies_to_object(&selection, object)
                {
                    self.scratch.push(id);
                }
            }
            let specific = self.list(specific_mark);

            let any_directives = self.kids[specific.range()]
                .iter()
                .any(|id| !self.sels[*id as usize].directives.is_empty());
            if any_directives {
                let children = self.flatten_fragments(inherited, object_def)?;
                if !children.is_empty() {
                    let id = self.push_sel(Selection {
                        kind: Kind::InlineFragment,
                        name: NONE,
                        alias: NONE,
                        type_condition: object_sym,
                        args: Span::EMPTY,
                        directives: fragment.directives,
                        children,
                    });
                    self.scratch.push(id);
                }

                for index in 0..specific.len as usize {
                    let (_, sub) = self.sel(specific, index);
                    let mut needs_wrapping = false;
                    let mut to_merge = Vec::new();
                    for parent_directive in fragment.directives.range() {
                        let parent_directive = self.directives[parent_directive];
                        let same_named = self.directives[sub.directives.range()]
                            .iter()
                            .find(|d| d.name == parent_directive.name)
                            .copied();
                        match same_named {
                            Some(existing)
                                if self.args_eq(existing.args, parent_directive.args) => {}
                            Some(_) => {
                                needs_wrapping = true;
                                break;
                            }
                            None => to_merge.push(parent_directive),
                        }
                    }

                    let children = self.flatten_fragments(sub.children, object_def)?;
                    if children.is_empty() {
                        continue;
                    }
                    let specific_fragment = Selection {
                        type_condition: object_sym,
                        children,
                        ..sub
                    };
                    if needs_wrapping {
                        let inner = self.push_sel(specific_fragment);
                        let mark = self.scratch.len();
                        self.scratch.push(inner);
                        let wrapper_children = self.list(mark);
                        let id = self.push_sel(Selection {
                            kind: Kind::InlineFragment,
                            name: NONE,
                            alias: NONE,
                            type_condition: object_sym,
                            args: Span::EMPTY,
                            directives: fragment.directives,
                            children: wrapper_children,
                        });
                        self.scratch.push(id);
                    } else {
                        let directives = match to_merge.is_empty() {
                            true => specific_fragment.directives,
                            false => {
                                let start = self.directives.len() as u32;
                                self.directives
                                    .extend_from_within(specific_fragment.directives.range());
                                self.directives.extend(to_merge);
                                Span {
                                    start,
                                    len: self.directives.len() as u32 - start,
                                }
                            }
                        };
                        let id = self.push_sel(Selection {
                            directives,
                            ..specific_fragment
                        });
                        self.scratch.push(id);
                    }
                }
                continue;
            }

            // one fragment with the fields and every applying fragment's selections
            let mut directives = fragment.directives;
            let items_mark = self.scratch.len();
            self.extend_scratch(inherited);
            for index in 0..specific.len as usize {
                let (_, sub) = self.sel(specific, index);
                directives = self.concat_directives(directives, sub.directives);
                self.extend_scratch(sub.children);
            }
            let merged = self.list(items_mark);
            let children = self.flatten_fragments(merged, object_def)?;
            if !children.is_empty() {
                let id = self.push_sel(Selection {
                    kind: Kind::InlineFragment,
                    name: NONE,
                    alias: NONE,
                    type_condition: object_sym,
                    args: Span::EMPTY,
                    directives,
                    children,
                });
                self.scratch.push(id);
            }
        }
        Ok(())
    }

    // ---- type_expand

    fn type_expand(&mut self, list: Span, type_def: &'a SupergraphDefinition) -> Result<Span> {
        let possible_objects = match type_def {
            SupergraphDefinition::Interface(interface) => Some(self.interface_objects(interface)?),
            _ => None,
        };

        let mark = self.scratch.len();
        for index in 0..list.len as usize {
            let (id, selection) = self.sel(list, index);
            match selection.kind {
                Kind::Field => {
                    if self.s(selection.name).starts_with("__") {
                        self.scratch.push(id);
                        continue;
                    }
                    let info = self
                        .field_info(type_def.fields(), selection.name)
                        .ok_or_else(|| NormalizationError::FieldNotFoundInType {
                            field_name: self.s(selection.name).to_string(),
                            type_name: type_def.name().to_string(),
                        })?;
                    if let Some(objects) = &possible_objects {
                        if self.expand_field(objects, type_def, id, selection)? {
                            continue;
                        }
                    }
                    if selection.children.is_empty() {
                        self.scratch.push(id);
                        continue;
                    }
                    let inner = info
                        .ty
                        .ok_or_else(|| NormalizationError::SchemaTypeNotFound {
                            type_name: info.field.field_type.inner_type().to_string(),
                        })?;
                    let children = self.type_expand(selection.children, inner)?;
                    let id = self.with_children(id, children);
                    self.scratch.push(id);
                }
                Kind::InlineFragment => {
                    let inner = match selection.type_condition == NONE {
                        true => Some(type_def),
                        false => self.defs[selection.type_condition as usize],
                    };
                    let id = match inner {
                        Some(inner) => {
                            let children = self.type_expand(selection.children, inner)?;
                            self.with_children(id, children)
                        }
                        None => id,
                    };
                    self.scratch.push(id);
                }
                Kind::FragmentSpread => self.scratch.push(id),
            }
        }
        Ok(self.finish(mark, list))
    }

    /// Sends the interface's field out as a fragment per object, when it has to. Whether it did.
    /// The objects of an interface, sorted by name.
    fn interface_objects(
        &mut self,
        interface: &'a crate::query_planner::state::supergraph_state::SupergraphInterfaceType,
    ) -> Result<std::rc::Rc<[&'a SupergraphObjectType]>> {
        let key = interface as *const _ as usize;
        if let Some(objects) = self.interface_objects.get(&key) {
            return Ok(objects.clone());
        }
        let state = self.state;
        // the subgraph has to know the interface, but it goes to every object implementing it
        let is_known = self.subgraph.is_none_or(|subgraph| {
            state
                .possible_types_in_subgraph(&interface.name, subgraph)
                .is_some()
        });
        let names = state
            .all_possible_types(&interface.name)
            .filter(|_| is_known)
            .ok_or_else(|| NormalizationError::PossibleTypesNotFound {
                type_name: interface.name.clone(),
            })?;
        let mut objects = Vec::with_capacity(names.len());
        for name in names {
            let Some(SupergraphDefinition::Object(object)) = state.definitions.get(name) else {
                return Err(NormalizationError::SchemaTypeNotFound {
                    type_name: name.clone(),
                });
            };
            objects.push(object);
        }
        objects.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        let objects: std::rc::Rc<[&'a SupergraphObjectType]> = objects.into();
        self.interface_objects.insert(key, objects.clone());
        Ok(objects)
    }

    /// Whether the interface's field has to go out as a fragment per object: an object
    /// doesn't have it or can't resolve it itself, or not every subgraph with the interface
    /// resolves it.
    fn should_expand_field(
        &self,
        objects: &[&'a SupergraphObjectType],
        type_def: &'a SupergraphDefinition,
        name: &str,
    ) -> bool {
        let SupergraphDefinition::Interface(interface) = type_def else {
            return false;
        };
        let state = self.state;
        let should_expand = objects.iter().any(|object| match object.fields.get(name) {
            None => true,
            Some(object_field) => object_field.join_field.iter().any(|jf| {
                jf.external
                    || jf.used_overridden
                    || jf
                        .override_value
                        .as_ref()
                        .is_some_and(|name| state.subgraph_exists_by_name(name))
            }),
        });

        if !should_expand {
            let subgraphs_with_interface = interface
                .join_type
                .iter()
                .filter(|jt| !jt.is_interface_object)
                .count();
            if subgraphs_with_interface <= 1 {
                return false;
            }
            let Some(field_def) = interface.fields.get(name) else {
                return false;
            };
            if field_def.join_field.is_empty() {
                return false;
            }
            let interface_object_in_graphs: Vec<_> = interface
                .join_type
                .iter()
                .filter(|jt| jt.is_interface_object)
                .map(|jt| &jt.graph_id)
                .collect();
            let is_interface_object_field =
                field_def.join_field.iter().all(|j| j.graph_id.is_none())
                    || field_def.join_field.iter().any(|jf| {
                        !jf.external
                            && jf
                                .graph_id
                                .as_ref()
                                .is_some_and(|g| interface_object_in_graphs.contains(&g))
                    });
            if is_interface_object_field {
                return false;
            }
            let subgraphs_resolving_field = field_def
                .join_field
                .iter()
                .filter(|jf| !jf.external && jf.graph_id.is_some())
                .count();
            if subgraphs_resolving_field >= subgraphs_with_interface {
                return false;
            }
        }
        true
    }

    fn expand_field(
        &mut self,
        objects: &[&'a SupergraphObjectType],
        type_def: &'a SupergraphDefinition,
        field_id: SelId,
        field: Selection,
    ) -> Result<bool> {
        let SupergraphDefinition::Interface(interface) = type_def else {
            return Ok(false);
        };
        let key = (interface as *const _ as usize, field.name);
        let expand = match self.expand.get(&key) {
            Some(expand) => *expand,
            None => {
                let expand = self.should_expand_field(objects, type_def, self.s(field.name));
                self.expand.insert(key, expand);
                expand
            }
        };
        if !expand {
            return Ok(false);
        }

        for object in objects {
            let mut new_field = field_id;
            if !field.children.is_empty() {
                if let Some(info) = self.field_info(&object.fields, field.name) {
                    let inner = info
                        .ty
                        .ok_or_else(|| NormalizationError::SchemaTypeNotFound {
                            type_name: info.field.field_type.inner_type().to_string(),
                        })?;
                    let children = self.type_expand(field.children, inner)?;
                    new_field = self.with_children(field_id, children);
                }
            }
            let mark = self.scratch.len();
            self.scratch.push(new_field);
            let children = self.list(mark);
            let type_condition = self.intern(&object.name);
            let id = self.push_sel(Selection {
                kind: Kind::InlineFragment,
                name: NONE,
                alias: NONE,
                type_condition,
                args: Span::EMPTY,
                directives: field.directives,
                children,
            });
            self.scratch.push(id);
        }
        Ok(true)
    }

    // ---- merge_inline_fragments

    fn merge_inline_fragments(
        &mut self,
        list: Span,
        parent: &'a SupergraphDefinition,
    ) -> Result<Span> {
        let mark = self.scratch.len();
        for index in 0..list.len as usize {
            let (id, selection) = self.sel(list, index);
            match selection.kind {
                Kind::Field => {
                    if self.s(selection.name).starts_with("__") || selection.children.is_empty() {
                        self.scratch.push(id);
                        continue;
                    }
                    let inner = self.field_type(parent, selection.name)?;
                    let children = self.merge_inline_fragments(selection.children, inner)?;
                    let id = self.with_children(id, children);
                    self.scratch.push(id);
                }
                Kind::InlineFragment => {
                    let child_parent = match selection.type_condition == NONE {
                        true => parent,
                        false => self.defs[selection.type_condition as usize].unwrap_or(parent),
                    };
                    let children = self.merge_inline_fragments(selection.children, child_parent)?;
                    let children = self.flatten_redundant_nested_inline_fragments(
                        selection.type_condition,
                        selection.directives,
                        children,
                    );

                    let mut merged = false;
                    for slot in mark..self.scratch.len() {
                        let existing_id = self.scratch[slot];
                        let existing = self.sels[existing_id as usize];
                        if existing.kind != Kind::InlineFragment
                            || existing.type_condition != selection.type_condition
                            || !self.directives_eq(selection.directives, existing.directives)
                        {
                            continue;
                        }
                        let combined_mark = self.scratch.len();
                        self.extend_scratch(existing.children);
                        self.extend_scratch(children);
                        let combined = self.list(combined_mark);
                        let combined = self.merge_inline_fragments(combined, child_parent)?;
                        self.scratch[slot] = self.with_children(existing_id, combined);
                        merged = true;
                        break;
                    }
                    if !merged {
                        let id = self.with_children(id, children);
                        self.scratch.push(id);
                    }
                }
                Kind::FragmentSpread => {}
            }
        }
        Ok(self.finish(mark, list))
    }

    fn flatten_redundant_nested_inline_fragments(
        &mut self,
        type_condition: Sym,
        directives: Span,
        list: Span,
    ) -> Span {
        let mark = self.scratch.len();
        for index in 0..list.len as usize {
            let (id, child) = self.sel(list, index);
            let flatten = child.kind == Kind::InlineFragment
                && child.type_condition == type_condition
                && (child.directives.is_empty()
                    || self.directives_eq(child.directives, directives));
            match flatten {
                true => self.extend_scratch(child.children),
                false => self.scratch.push(id),
            }
        }
        self.finish(mark, list)
    }

    // ---- flatten_matching_object_fragments

    fn flatten_matching_object_fragments(
        &mut self,
        list: Span,
        parent: &'a SupergraphDefinition,
    ) -> Result<Span> {
        let mark = self.scratch.len();
        for index in 0..list.len as usize {
            let (id, selection) = self.sel(list, index);
            match selection.kind {
                Kind::Field => {
                    if self.s(selection.name).starts_with("__") || selection.children.is_empty() {
                        self.scratch.push(id);
                        continue;
                    }
                    let inner = self.field_type(parent, selection.name)?;
                    let mut children =
                        self.flatten_matching_object_fragments(selection.children, inner)?;
                    if matches!(inner, SupergraphDefinition::Object(_)) {
                        children = self.flatten_field_selection_set(children, inner.name());
                    }
                    let id = self.with_children(id, children);
                    self.scratch.push(id);
                }
                Kind::InlineFragment => {
                    let child_parent = match selection.type_condition == NONE {
                        true => parent,
                        false => self.defs[selection.type_condition as usize].unwrap_or(parent),
                    };
                    let children =
                        self.flatten_matching_object_fragments(selection.children, child_parent)?;
                    let id = self.with_children(id, children);
                    self.scratch.push(id);
                }
                Kind::FragmentSpread => self.scratch.push(id),
            }
        }
        Ok(self.finish(mark, list))
    }

    fn flatten_field_selection_set(&mut self, list: Span, object: &str) -> Span {
        let mark = self.scratch.len();
        for index in 0..list.len as usize {
            let (id, selection) = self.sel(list, index);
            let flatten = selection.kind == Kind::InlineFragment
                && selection.directives.is_empty()
                && selection.type_condition != NONE
                && self.s(selection.type_condition) == object
                && !self.kids[selection.children.range()].iter().any(|child| {
                    let child = self.sels[*child as usize];
                    child.kind == Kind::Field && Some(child.name) == self.typename
                });
            match flatten {
                true => self.extend_scratch(selection.children),
                false => self.scratch.push(id),
            }
        }
        self.finish(mark, list)
    }

    // ---- merge_fields

    fn fields_equal(&self, a: &Selection, b: &Selection) -> bool {
        a.alias == b.alias
            && a.name == b.name
            && self.args_eq(a.args, b.args)
            && self.directives_eq(a.directives, b.directives)
    }

    fn merge_fields(&mut self, list: Span) -> Result<Span> {
        let mark = self.scratch.len();
        for index in 0..list.len as usize {
            let (id, selection) = self.sel(list, index);
            match selection.kind {
                Kind::Field => {
                    let children = self.merge_fields(selection.children)?;
                    let mut merged = false;
                    for slot in mark..self.scratch.len() {
                        let existing_id = self.scratch[slot];
                        let existing = self.sels[existing_id as usize];
                        if existing.kind != Kind::Field || !self.fields_equal(&selection, &existing)
                        {
                            continue;
                        }
                        let combined_mark = self.scratch.len();
                        self.extend_scratch(existing.children);
                        self.extend_scratch(children);
                        let combined = self.list(combined_mark);
                        let combined = self.merge_fields(combined)?;
                        self.scratch[slot] = self.with_children(existing_id, combined);
                        merged = true;
                        break;
                    }
                    if !merged {
                        let id = self.with_children(id, children);
                        self.scratch.push(id);
                    }
                }
                Kind::InlineFragment => {
                    let children = self.merge_fields(selection.children)?;
                    let id = self.with_children(id, children);
                    self.scratch.push(id);
                }
                Kind::FragmentSpread => {}
            }
        }
        Ok(self.finish(mark, list))
    }

    // ---- drop_duplicated_fields: a leaf field the list already has. Only into inline
    // fragments, never inside a field's selections.

    fn drop_duplicated_fields(&mut self, list: Span) -> Span {
        if list.len < 2 {
            return list;
        }
        let mark = self.scratch.len();
        for index in 0..list.len as usize {
            let (id, selection) = self.sel(list, index);
            match selection.kind {
                Kind::Field => {
                    let duplicate = self.scratch[mark..].iter().any(|kept| {
                        let kept = self.sels[*kept as usize];
                        kept.kind == Kind::Field
                            && kept.name == selection.name
                            && kept.alias == selection.alias
                            && self.args_eq(kept.args, selection.args)
                            && self.directives_eq(kept.directives, selection.directives)
                            && kept.children.is_empty()
                            && selection.children.is_empty()
                    });
                    if !duplicate {
                        self.scratch.push(id);
                    }
                }
                Kind::InlineFragment => {
                    let children = self.drop_duplicated_fields(selection.children);
                    let id = self.with_children(id, children);
                    self.scratch.push(id);
                }
                Kind::FragmentSpread => self.scratch.push(id),
            }
        }
        self.finish(mark, list)
    }
}
