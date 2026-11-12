//! `graphql_tools::validation::rules::OverlappingFieldsCanBeMerged`, flat.
//!
//! Same walk as the tree's, step for step, quirks too (see the comments). Names are `Sym`s, so
//! comparing them is comparing numbers, and strings get built only when there's a conflict.

use std::collections::HashMap;

use graphql_tools::flat::{Fragment, Kind, ListId, Pos, Selection, Sym, NONE};
use graphql_tools::static_graphql::query::Type;
use graphql_tools::static_graphql::schema::{Field as FieldDefinition, TypeDefinition};

use crate::pipeline::validation::flat::visit::{
    error, pos, values_equal, BoxedVisitor, Ctx, Errors, FlatRule, Visitor,
};

const CODE: &str = "OverlappingFieldsCanBeMerged";

pub struct OverlappingFieldsCanBeMerged;

impl FlatRule for OverlappingFieldsCanBeMerged {
    fn visitor<'a>(&self) -> BoxedVisitor<'a> {
        Box::new(Rule {
            named_fragments: HashMap::new(),
            compared_fragments: HashMap::new(),
        })
    }

    fn error_code(&self) -> &'static str {
        CODE
    }
}

struct Conflict(Reason, Vec<Pos>, Vec<Pos>);

#[derive(Clone)]
struct Reason(Sym, Message);

#[derive(Clone)]
enum Message {
    Text(String),
    Nested(Vec<Reason>),
}

#[derive(Clone, Copy)]
struct AstAndDef<'a>(
    Option<&'a TypeDefinition>,
    &'a Selection,
    Option<&'a FieldDefinition>,
);

/// Response name to the fields under it, in the order the names first showed up.
#[derive(Default)]
struct FieldMap<'a> {
    index: HashMap<Sym, usize>,
    groups: Vec<(Sym, Vec<AstAndDef<'a>>)>,
}

impl<'a> FieldMap<'a> {
    fn push(&mut self, name: Sym, item: AstAndDef<'a>) {
        let groups = &mut self.groups;
        let i = *self.index.entry(name).or_insert_with(|| {
            groups.push((name, Vec::new()));
            groups.len() - 1
        });
        self.groups[i].1.push(item);
    }

    fn get(&self, name: Sym) -> Option<&[AstAndDef<'a>]> {
        self.index.get(&name).map(|&i| self.groups[i].1.as_slice())
    }
}

struct Rule<'a> {
    /// By name, the last one wins, like the tree's.
    named_fragments: HashMap<Sym, &'a Fragment>,
    /// The tree's `PairSet`: both (a, b) and (b, a) go in.
    compared_fragments: HashMap<(Sym, Sym), bool>,
}

impl<'a> Rule<'a> {
    fn find_conflicts_within_selection_set(
        &mut self,
        ctx: &Ctx<'a>,
        parent_type: Option<&'a TypeDefinition>,
        list: ListId,
        visited: &mut Vec<Sym>,
    ) -> Vec<Conflict> {
        let mut conflicts = Vec::new();
        let (field_map, fragment_names) = fields_and_fragment_names(ctx, parent_type, list);

        // (A)
        self.collect_conflicts_within(ctx, &mut conflicts, &field_map, visited);

        // (B) and (C)
        for (i, &frag_name1) in fragment_names.iter().enumerate() {
            self.collect_conflicts_between_fields_and_fragment(
                ctx,
                &mut conflicts,
                &field_map,
                frag_name1,
                false,
                visited,
            );
            for &frag_name2 in &fragment_names[i + 1..] {
                self.collect_conflicts_between_fragments(
                    ctx,
                    &mut conflicts,
                    frag_name1,
                    frag_name2,
                    false,
                    visited,
                );
            }
        }

        conflicts
    }

    fn collect_conflicts_within(
        &mut self,
        ctx: &Ctx<'a>,
        conflicts: &mut Vec<Conflict>,
        field_map: &FieldMap<'a>,
        visited: &mut Vec<Sym>,
    ) {
        for (name, fields) in &field_map.groups {
            for (index, first) in fields.iter().enumerate() {
                for second in &fields[index + 1..] {
                    if let Some(c) = self.find_conflict(ctx, *name, first, second, false, visited) {
                        conflicts.push(c);
                    }
                }
            }
        }
    }

    fn find_conflict(
        &mut self,
        ctx: &Ctx<'a>,
        out_name: Sym,
        first: &AstAndDef<'a>,
        second: &AstAndDef<'a>,
        parents_mutually_exclusive: bool,
        visited: &mut Vec<Sym>,
    ) -> Option<Conflict> {
        let doc = ctx.doc;
        let AstAndDef(parent_type1, field1, field1_def) = *first;
        let AstAndDef(parent_type2, field2, field2_def) = *second;

        let mut mutually_exclusive = parents_mutually_exclusive;
        if !parents_mutually_exclusive {
            if let (Some(p1), Some(p2)) = (parent_type1, parent_type2) {
                mutually_exclusive =
                    p1.name() != p2.name() && p1.is_object_type() && p2.is_object_type();
            }
        }

        let conflict = |text: String| {
            Some(Conflict(
                Reason(out_name, Message::Text(text)),
                vec![field1.pos],
                vec![field2.pos],
            ))
        };

        if !mutually_exclusive {
            if field1.name != field2.name {
                return conflict(format!(
                    "\"{}\" and \"{}\" are different fields",
                    doc.str(field1.name),
                    doc.str(field2.name)
                ));
            }
            if !same_arguments(ctx, field1, field2) {
                return conflict("they have differing arguments".to_string());
            }
        }

        let t1 = field1_def.map(|def| &def.field_type);
        let t2 = field2_def.map(|def| &def.field_type);

        if let (Some(t1), Some(t2)) = (t1, t2) {
            if is_type_conflict(ctx, t1, t2) {
                return conflict(format!(
                    "they return conflicting types \"{}\" and \"{}\"",
                    t1, t2
                ));
            }
        }

        if !doc.is_empty(field1.children) && !doc.is_empty(field2.children) {
            let conflicts = self.find_conflicts_between_sub_selection_sets(
                ctx,
                mutually_exclusive,
                t1.and_then(|t| ctx.type_by_name(t.inner_type())),
                field1.children,
                t2.and_then(|t| ctx.type_by_name(t.inner_type())),
                field2.children,
                visited,
            );
            // the tree passes field1's position twice, so do we
            return subfield_conflicts(conflicts, out_name, field1.pos, field1.pos);
        }

        None
    }

    #[allow(clippy::too_many_arguments)]
    fn find_conflicts_between_sub_selection_sets(
        &mut self,
        ctx: &Ctx<'a>,
        mutually_exclusive: bool,
        parent_type1: Option<&'a TypeDefinition>,
        list1: ListId,
        parent_type2: Option<&'a TypeDefinition>,
        list2: ListId,
        visited: &mut Vec<Sym>,
    ) -> Vec<Conflict> {
        let mut conflicts = Vec::new();
        let (field_map1, fragment_names1) = fields_and_fragment_names(ctx, parent_type1, list1);
        let (field_map2, fragment_names2) = fields_and_fragment_names(ctx, parent_type2, list2);

        // (H)
        self.collect_conflicts_between(
            ctx,
            &mut conflicts,
            mutually_exclusive,
            &field_map1,
            &field_map2,
            visited,
        );

        // (I)
        for &name in &fragment_names2 {
            self.collect_conflicts_between_fields_and_fragment(
                ctx,
                &mut conflicts,
                &field_map1,
                name,
                mutually_exclusive,
                visited,
            );
        }
        for &name in &fragment_names1 {
            self.collect_conflicts_between_fields_and_fragment(
                ctx,
                &mut conflicts,
                &field_map2,
                name,
                mutually_exclusive,
                visited,
            );
        }

        // (J)
        for &name1 in &fragment_names1 {
            for &name2 in &fragment_names2 {
                self.collect_conflicts_between_fragments(
                    ctx,
                    &mut conflicts,
                    name1,
                    name2,
                    mutually_exclusive,
                    visited,
                );
            }
        }

        conflicts
    }

    fn collect_conflicts_between_fields_and_fragment(
        &mut self,
        ctx: &Ctx<'a>,
        conflicts: &mut Vec<Conflict>,
        field_map: &FieldMap<'a>,
        fragment_name: Sym,
        mutually_exclusive: bool,
        visited: &mut Vec<Sym>,
    ) {
        let Some(&fragment) = self.named_fragments.get(&fragment_name) else {
            return;
        };
        let (field_map2, fragment_names2) = referenced_fields_and_fragment_names(ctx, fragment);

        if fragment_names2.contains(&fragment_name) {
            return;
        }

        self.collect_conflicts_between(
            ctx,
            conflicts,
            mutually_exclusive,
            field_map,
            &field_map2,
            visited,
        );

        for &name2 in &fragment_names2 {
            // `return`, not `continue`, like the tree
            if visited.contains(&name2) {
                return;
            }
            visited.push(name2);
            self.collect_conflicts_between_fields_and_fragment(
                ctx,
                conflicts,
                field_map,
                name2,
                mutually_exclusive,
                visited,
            );
        }
    }

    fn collect_conflicts_between_fragments(
        &mut self,
        ctx: &Ctx<'a>,
        conflicts: &mut Vec<Conflict>,
        name1: Sym,
        name2: Sym,
        mutually_exclusive: bool,
        visited: &mut Vec<Sym>,
    ) {
        if name1 == name2 {
            return;
        }

        // the tree's `PairSet::contains`
        if let Some(&was_exclusive) = self.compared_fragments.get(&(name1, name2)) {
            if mutually_exclusive || !was_exclusive {
                return;
            }
        }
        self.compared_fragments
            .insert((name1, name2), mutually_exclusive);
        self.compared_fragments
            .insert((name2, name1), mutually_exclusive);

        let (Some(&fragment1), Some(&fragment2)) = (
            self.named_fragments.get(&name1),
            self.named_fragments.get(&name2),
        ) else {
            return;
        };

        let (field_map1, fragment_names1) = referenced_fields_and_fragment_names(ctx, fragment1);
        let (field_map2, fragment_names2) = referenced_fields_and_fragment_names(ctx, fragment2);

        // (F)
        self.collect_conflicts_between(
            ctx,
            conflicts,
            mutually_exclusive,
            &field_map1,
            &field_map2,
            visited,
        );

        // (G)
        for &n2 in &fragment_names2 {
            self.collect_conflicts_between_fragments(
                ctx,
                conflicts,
                name1,
                n2,
                mutually_exclusive,
                visited,
            );
        }
        for &n1 in &fragment_names1 {
            self.collect_conflicts_between_fragments(
                ctx,
                conflicts,
                n1,
                name2,
                mutually_exclusive,
                visited,
            );
        }
    }

    fn collect_conflicts_between(
        &mut self,
        ctx: &Ctx<'a>,
        conflicts: &mut Vec<Conflict>,
        mutually_exclusive: bool,
        field_map1: &FieldMap<'a>,
        field_map2: &FieldMap<'a>,
        visited: &mut Vec<Sym>,
    ) {
        for (name, fields1) in &field_map1.groups {
            if let Some(fields2) = field_map2.get(*name) {
                for field1 in fields1 {
                    for field2 in fields2 {
                        if let Some(c) = self.find_conflict(
                            ctx,
                            *name,
                            field1,
                            field2,
                            mutually_exclusive,
                            visited,
                        ) {
                            conflicts.push(c);
                        }
                    }
                }
            }
        }
    }
}

/// The tree compares arguments by name, first match wins, values by `Value::compare`.
fn same_arguments(ctx: &Ctx<'_>, f1: &Selection, f2: &Selection) -> bool {
    let doc = ctx.doc;
    let (args1, args2) = (doc.args_of(f1.args), doc.args_of(f2.args));
    if args1.len() != args2.len() {
        return false;
    }
    args1.iter().all(|a1| {
        args2
            .iter()
            .find(|a2| a2.name == a1.name)
            .is_some_and(|a2| values_equal(doc, a1.value, a2.value))
    })
}

fn is_type_conflict(ctx: &Ctx<'_>, t1: &Type, t2: &Type) -> bool {
    match (t1, t2) {
        (Type::ListType(a), Type::ListType(b)) => is_type_conflict(ctx, a, b),
        (Type::ListType(_), _) | (_, Type::ListType(_)) => true,
        (Type::NonNullType(a), Type::NonNullType(b)) => is_type_conflict(ctx, a, b),
        (Type::NonNullType(_), _) | (_, Type::NonNullType(_)) => true,
        _ => {
            let leaf = |t: &Type| {
                ctx.type_by_name(t.inner_type())
                    .is_some_and(|t| t.is_leaf_type())
            };
            (leaf(t1) || leaf(t2)) && t1 != t2
        }
    }
}

fn subfield_conflicts(
    conflicts: Vec<Conflict>,
    out_name: Sym,
    f1_pos: Pos,
    f2_pos: Pos,
) -> Option<Conflict> {
    if conflicts.is_empty() {
        return None;
    }
    // the tree adds the first-side positions of the nested ones to both sides
    let nested = || conflicts.iter().flat_map(|c| c.1.iter().copied());
    let p1 = std::iter::once(f1_pos).chain(nested()).collect();
    let p2 = std::iter::once(f2_pos).chain(nested()).collect();
    let reasons = conflicts.into_iter().map(|c| c.0).collect();
    Some(Conflict(Reason(out_name, Message::Nested(reasons)), p1, p2))
}

fn referenced_fields_and_fragment_names<'a>(
    ctx: &Ctx<'a>,
    fragment: &'a Fragment,
) -> (FieldMap<'a>, Vec<Sym>) {
    let fragment_type = ctx.type_by_name(ctx.str(fragment.type_condition));
    fields_and_fragment_names(ctx, fragment_type, fragment.selection_set)
}

fn fields_and_fragment_names<'a>(
    ctx: &Ctx<'a>,
    parent_type: Option<&'a TypeDefinition>,
    list: ListId,
) -> (FieldMap<'a>, Vec<Sym>) {
    let mut fields = FieldMap::default();
    let mut fragment_names = Vec::new();
    collect_fields_and_fragment_names(ctx, parent_type, list, &mut fields, &mut fragment_names);
    (fields, fragment_names)
}

fn collect_fields_and_fragment_names<'a>(
    ctx: &Ctx<'a>,
    parent_type: Option<&'a TypeDefinition>,
    list: ListId,
    fields: &mut FieldMap<'a>,
    fragment_names: &mut Vec<Sym>,
) {
    let doc = ctx.doc;
    for &id in doc.items(list) {
        let s = doc.selection(id);
        match s.kind {
            Kind::Field => {
                let def = parent_type.and_then(|t| t.field_by_name(doc.str(s.name)));
                let out_name = if s.alias != NONE { s.alias } else { s.name };
                fields.push(out_name, AstAndDef(parent_type, s, def));
            }
            Kind::FragmentSpread => {
                if !fragment_names.contains(&s.name) {
                    fragment_names.push(s.name);
                }
            }
            Kind::InlineFragment => {
                let fragment_type = doc
                    .opt_str(s.type_condition)
                    .and_then(|name| ctx.type_by_name(name))
                    .or(parent_type);
                collect_fields_and_fragment_names(
                    ctx,
                    fragment_type,
                    s.children,
                    fields,
                    fragment_names,
                );
            }
        }
    }
}

fn error_message(ctx: &Ctx<'_>, Reason(name, message): &Reason) -> String {
    format!(
        r#"Fields "{}" conflict because {}. Use different aliases on the fields to fetch both if this was intentional."#,
        ctx.str(*name),
        format_reason(ctx, message)
    )
}

fn format_reason(ctx: &Ctx<'_>, message: &Message) -> String {
    match message {
        Message::Text(text) => text.clone(),
        Message::Nested(nested) => nested
            .iter()
            .map(|Reason(name, sub)| {
                format!(
                    r#"subfields "{}" conflict because {}"#,
                    ctx.str(*name),
                    format_reason(ctx, sub)
                )
            })
            .collect::<Vec<_>>()
            .join(" and "),
    }
}

impl<'a> Visitor<'a> for Rule<'a> {
    fn enter_document(&mut self, ctx: &mut Ctx<'a>, _: &mut Errors) {
        for fragment in ctx.doc.fragments() {
            self.named_fragments.insert(fragment.name, fragment);
        }
    }

    fn enter_selection_set(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, list: ListId) {
        let parent_type = ctx.current_parent_type();
        let mut visited = Vec::new();
        let conflicts =
            self.find_conflicts_within_selection_set(ctx, parent_type, list, &mut visited);
        for Conflict(reason, p1, p2) in conflicts {
            let locations = p1.into_iter().chain(p2).map(pos).collect();
            errors.report_error(error(CODE, error_message(ctx, &reason), locations));
        }
    }
}
