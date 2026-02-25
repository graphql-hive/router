use crate::parser::query::{Definition, TypeCondition};
use crate::parser::Pos;

use super::ValidationRule;
use crate::ast::{visit_document, OperationVisitor, OperationVisitorContext};
use crate::static_graphql::query::*;
use crate::static_graphql::schema::{
    Document as SchemaDocument, Field as FieldDefinition, TypeDefinition,
};
use crate::validation::utils::{ValidationError, ValidationErrorContext};
use std::borrow::Borrow;
use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::Hash;
/// Overlapping fields can be merged
///
/// A selection set is only valid if all fields (including spreading any
/// fragments) either correspond to distinct response names or can be merged
/// without ambiguity.
///
/// See https://spec.graphql.org/draft/#sec-Field-Selection-Merging
pub struct OverlappingFieldsCanBeMerged<'a> {
    named_fragments: HashMap<&'a str, &'a FragmentDefinition>,
    compared_fragments: PairSet<'a>,
}

/**
 * Algorithm:
 *
 * Conflicts occur when two fields exist in a query which will produce the same
 * response name, but represent differing values, thus creating a conflict.
 * The algorithm below finds all conflicts via making a series of comparisons
 * between fields. In order to compare as few fields as possible, this makes
 * a series of comparisons "within" sets of fields and "between" sets of fields.
 *
 * Given any selection set, a collection produces both a set of fields by
 * also including all inline fragments, as well as a list of fragments
 * referenced by fragment spreads.
 *
 * A) Each selection set represented in the document first compares "within" its
 * collected set of fields, finding any conflicts between every pair of
 * overlapping fields.
 * Note: This is the *only time* that a the fields "within" a set are compared
 * to each other. After this only fields "between" sets are compared.
 *
 * B) Also, if any fragment is referenced in a selection set, then a
 * comparison is made "between" the original set of fields and the
 * referenced fragment.
 *
 * C) Also, if multiple fragments are referenced, then comparisons
 * are made "between" each referenced fragment.
 *
 * D) When comparing "between" a set of fields and a referenced fragment, first
 * a comparison is made between each field in the original set of fields and
 * each field in the the referenced set of fields.
 *
 * E) Also, if any fragment is referenced in the referenced selection set,
 * then a comparison is made "between" the original set of fields and the
 * referenced fragment (recursively referring to step D).
 *
 * F) When comparing "between" two fragments, first a comparison is made between
 * each field in the first referenced set of fields and each field in the the
 * second referenced set of fields.
 *
 * G) Also, any fragments referenced by the first must be compared to the
 * second, and any fragments referenced by the second must be compared to the
 * first (recursively referring to step F).
 *
 * H) When comparing two fields, if both have selection sets, then a comparison
 * is made "between" both selection sets, first comparing the set of fields in
 * the first selection set with the set of fields in the second.
 *
 * I) Also, if any fragment is referenced in either selection set, then a
 * comparison is made "between" the other set of fields and the
 * referenced fragment.
 *
 * J) Also, if two fragments are referenced in both selection sets, then a
 * comparison is made "between" the two fragments.
 *
 */

#[derive(Debug)]
struct Conflict(ConflictReason, Vec<Pos>, Vec<Pos>);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ConflictReason(String, ConflictReasonMessage);

#[derive(Debug)]
struct AstAndDef<'a>(
    Option<&'a TypeDefinition>,
    &'a Field,
    Option<&'a FieldDefinition>,
);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ConflictReasonMessage {
    Message(String),
    Nested(Vec<ConflictReason>),
}

struct PairSet<'a> {
    data: HashMap<&'a str, HashMap<&'a str, bool>>,
}

struct OrderedMap<K, V> {
    data: HashMap<K, V>,
    insert_order: Vec<K>,
}

struct OrderedMapIter<'a, K: 'a, V: 'a> {
    map: &'a HashMap<K, V>,
    inner: ::std::slice::Iter<'a, K>,
}

impl<K: Eq + Hash + Clone, V> OrderedMap<K, V> {
    fn new() -> OrderedMap<K, V> {
        OrderedMap {
            data: HashMap::new(),
            insert_order: Vec::new(),
        }
    }

    fn iter<'a>(&'a self) -> OrderedMapIter<'a, K, V> {
        OrderedMapIter {
            map: &self.data,
            inner: self.insert_order.iter(),
        }
    }

    fn get<Q>(&self, k: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: ?Sized + Hash + Eq,
    {
        self.data.get(k)
    }

    fn get_mut<Q>(&mut self, k: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
        Q: ?Sized + Hash + Eq,
    {
        self.data.get_mut(k)
    }

    fn contains_key<Q>(&self, k: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: ?Sized + Hash + Eq,
    {
        self.data.contains_key(k)
    }

    fn insert(&mut self, k: K, v: V) -> Option<V> {
        let result = self.data.insert(k.clone(), v);
        if result.is_none() {
            self.insert_order.push(k);
        }
        result
    }
}

impl<'a, K: Eq + Hash + 'a, V: 'a> Iterator for OrderedMapIter<'a, K, V> {
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .and_then(|key| self.map.get(key).map(|value| (key, value)))
    }
}

impl<'a> PairSet<'a> {
    fn new() -> PairSet<'a> {
        PairSet {
            data: HashMap::new(),
        }
    }

    pub fn contains(&self, a: &str, b: &str, mutex: bool) -> bool {
        if let Some(result) = self.data.get(a).and_then(|s| s.get(b)) {
            if !mutex {
                !result
            } else {
                true
            }
        } else {
            false
        }
    }

    pub fn insert(&mut self, a: &'a str, b: &'a str, mutex: bool) {
        self.data.entry(a).or_default().insert(b, mutex);

        self.data.entry(b).or_default().insert(a, mutex);
    }
}

impl<'a> Default for OverlappingFieldsCanBeMerged<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> OverlappingFieldsCanBeMerged<'a> {
    pub fn new() -> Self {
        Self {
            named_fragments: HashMap::new(),
            compared_fragments: PairSet::new(),
        }
    }

    // Find all conflicts found "within" a selection set, including those found
    // via spreading in fragments. Called when visiting each SelectionSet in the
    // GraphQL Document.
    fn find_conflicts_within_selection_set(
        &mut self,
        schema: &'a SchemaDocument,
        parent_type: Option<&'a TypeDefinition>,
        selection_set: &'a SelectionSet,
        visited_fragments: &mut Vec<&'a str>,
    ) -> Vec<Conflict> {
        let mut conflicts = Vec::<Conflict>::new();

        let (field_map, fragment_names) =
            self.get_fields_and_fragment_names(schema, parent_type, selection_set);

        // (A) Find find all conflicts "within" the fields of this selection set.
        // Note: this is the *only place* `collect_conflicts_within` is called.
        self.collect_conflicts_within(schema, &mut conflicts, &field_map, visited_fragments);

        // (B) Then collect conflicts between these fields and those represented by
        // each spread fragment name found.
        for (i, frag_name1) in fragment_names.iter().enumerate() {
            self.collect_conflicts_between_fields_and_fragment(
                schema,
                &mut conflicts,
                &field_map,
                frag_name1,
                false,
                visited_fragments,
            );

            // (C) Then compare this fragment with all other fragments found in this
            // selection set to collect conflicts between fragments spread together.
            // This compares each item in the list of fragment names to every other
            // item in that same list (except for itself).
            for frag_name2 in &fragment_names[i + 1..] {
                self.collect_conflicts_between_fragments(
                    schema,
                    &mut conflicts,
                    frag_name1,
                    frag_name2,
                    false,
                    visited_fragments,
                );
            }
        }

        conflicts
    }

    // Collect all Conflicts "within" one collection of fields.
    fn collect_conflicts_within(
        &mut self,
        schema: &'a SchemaDocument,
        conflicts: &mut Vec<Conflict>,
        field_map: &OrderedMap<&'a str, Vec<AstAndDef<'a>>>,
        visited_fragments: &mut Vec<&'a str>,
    ) {
        // A field map is a keyed collection, where each key represents a response
        // name and the value at that key is a list of all fields which provide that
        // response name. For every response name, if there are multiple fields, they
        // must be compared to find a potential conflict.
        for (out_field_name, fields) in field_map.iter() {
            // This compares every field in the list to every other field in this list
            // (except to itself). If the list only has one item, nothing needs to
            // be compared.
            for (index, first) in fields.iter().enumerate() {
                for second in &fields[index + 1..] {
                    if let Some(conflict) = self.find_conflict(
                        schema,
                        out_field_name,
                        first,
                        second,
                        false, // within one collection is never mutually exclusive
                        visited_fragments,
                    ) {
                        conflicts.push(conflict)
                    }
                }
            }
        }
    }

    fn is_same_arguments(&self, f1_args: &[(String, Value)], f2_args: &[(String, Value)]) -> bool {
        if f1_args.len() != f2_args.len() {
            return false;
        }

        f1_args.iter().all(|(n1, v1)| {
            if let Some((_, v2)) = f2_args.iter().find(|&(n2, _)| n1.eq(n2)) {
                v1.compare(v2)
            } else {
                false
            }
        })
    }

    // Two types conflict if both types could not apply to a value simultaneously.
    // Composite types are ignored as their individual field types will be compared
    // later recursively. However List and Non-Null types must match.
    fn is_type_conflict(schema: &SchemaDocument, t1: &Type, t2: &Type) -> bool {
        if let Type::ListType(t1) = t1 {
            if let Type::ListType(t2) = t2 {
                return Self::is_type_conflict(schema, t1, t2);
            } else {
                return true;
            }
        }

        if let Type::ListType(_) = t2 {
            return true;
        }

        if let Type::NonNullType(t1) = t1 {
            if let Type::NonNullType(t2) = t2 {
                return Self::is_type_conflict(schema, t1, t2);
            } else {
                return true;
            }
        }

        if let Type::NonNullType(_) = t2 {
            return true;
        }

        let schema_type1 = schema.type_by_name(t1.inner_type());
        let schema_type2 = schema.type_by_name(t2.inner_type());

        if schema_type1.map(|t| t.is_leaf_type()).unwrap_or(false)
            || schema_type2.map(|t| t.is_leaf_type()).unwrap_or(false)
        {
            t1 != t2
        } else {
            false
        }
    }

    // Determines if there is a conflict between two particular fields, including
    // comparing their sub-fields.
    fn find_conflict(
        &mut self,
        schema: &'a SchemaDocument,
        out_field_name: &str,
        first: &AstAndDef<'a>,
        second: &AstAndDef<'a>,
        parents_mutually_exclusive: bool,
        visited_fragments: &mut Vec<&'a str>,
    ) -> Option<Conflict> {
        let AstAndDef(parent_type1, field1, field1_def) = *first;
        let AstAndDef(parent_type2, field2, field2_def) = *second;

        // If it is known that two fields could not possibly apply at the same
        // time, due to the parent types, then it is safe to permit them to diverge
        // in aliased field or arguments used as they will not present any ambiguity
        // by differing.
        // It is known that two parent types could never overlap if they are
        // different Object types. Interface or Union types might overlap - if not
        // in the current state of the schema, then perhaps in some future version,
        // thus may not safely diverge.

        let (parent_type1_props, parent_type2_props) = (
            parent_type1.map(|t| (t.name(), t.is_object_type())),
            parent_type2.map(|t| (t.name(), t.is_object_type())),
        );

        let mut mutually_exclusive = parents_mutually_exclusive;

        if !parents_mutually_exclusive {
            if let (
                Some((parent_type1_name, parent_type1_is_object)),
                Some((parent_type2_name, parent_type2_is_object)),
            ) = (parent_type1_props, parent_type2_props)
            {
                mutually_exclusive = parent_type1_name != parent_type2_name
                    && parent_type1_is_object
                    && parent_type2_is_object;
            }
        }

        if !mutually_exclusive {
            let name1 = &field1.name;
            let name2 = &field2.name;

            if name1 != name2 {
                return Some(Conflict(
                    ConflictReason(
                        out_field_name.to_string(),
                        ConflictReasonMessage::Message(format!(
                            "\"{}\" and \"{}\" are different fields",
                            name1, name2
                        )),
                    ),
                    vec![field1.position],
                    vec![field2.position],
                ));
            }

            if !self.is_same_arguments(&field1.arguments, &field2.arguments) {
                return Some(Conflict(
                    ConflictReason(
                        out_field_name.to_string(),
                        ConflictReasonMessage::Message("they have differing arguments".to_string()),
                    ),
                    vec![field1.position],
                    vec![field2.position],
                ));
            }
        }

        let t1 = field1_def.as_ref().map(|def| &def.field_type);
        let t2 = field2_def.as_ref().map(|def| &def.field_type);

        if let (Some(t1), Some(t2)) = (t1, t2) {
            if Self::is_type_conflict(schema, t1, t2) {
                return Some(Conflict(
                    ConflictReason(
                        out_field_name.to_owned(),
                        ConflictReasonMessage::Message(format!(
                            "they return conflicting types \"{}\" and \"{}\"",
                            t1, t2
                        )),
                    ),
                    vec![field1.position],
                    vec![field2.position],
                ));
            }
        }

        // Collect and compare sub-fields. Use the same "visited fragment names" list
        // for both collections so fields in a fragment reference are never
        // compared to themselves.
        if !field1.selection_set.items.is_empty() && !field2.selection_set.items.is_empty() {
            let conflicts = self.find_conflicts_between_sub_selection_sets(
                schema,
                mutually_exclusive,
                t1.map(|v| v.inner_type()),
                &field1.selection_set,
                t2.map(|v| v.inner_type()),
                &field2.selection_set,
                visited_fragments,
            );

            return self.subfield_conflicts(
                &conflicts,
                out_field_name,
                field1.position,
                field1.position,
            );
        }

        None
    }

    fn subfield_conflicts(
        &self,
        conflicts: &[Conflict],
        out_field_name: &str,
        f1_pos: Pos,
        f2_pos: Pos,
    ) -> Option<Conflict> {
        if conflicts.is_empty() {
            return None;
        }

        Some(Conflict(
            ConflictReason(
                out_field_name.to_string(),
                ConflictReasonMessage::Nested(conflicts.iter().map(|v| v.0.clone()).collect()),
            ),
            vec![f1_pos]
                .into_iter()
                .chain(conflicts.iter().flat_map(|v| v.1.clone()))
                .collect(),
            vec![f2_pos]
                .into_iter()
                .chain(conflicts.iter().flat_map(|v| v.1.clone()))
                .collect(),
        ))
    }

    // Find all conflicts found between two selection sets, including those found
    // via spreading in fragments. Called when determining if conflicts exist
    // between the sub-fields of two overlapping fields.
    #[allow(clippy::too_many_arguments)]
    fn find_conflicts_between_sub_selection_sets(
        &mut self,
        schema: &'a SchemaDocument,
        mutually_exclusive: bool,
        parent_type_name1: Option<&str>,
        selection_set1: &'a SelectionSet,
        parent_type_name2: Option<&str>,
        selection_set2: &'a SelectionSet,
        visited_fragments: &mut Vec<&'a str>,
    ) -> Vec<Conflict> {
        let mut conflicts = Vec::<Conflict>::new();
        let parent_type1 = parent_type_name1.and_then(|t| schema.type_by_name(t));
        let parent_type2 = parent_type_name2.and_then(|t| schema.type_by_name(t));

        let (field_map1, fragment_names1) =
            self.get_fields_and_fragment_names(schema, parent_type1, selection_set1);
        let (field_map2, fragment_names2) =
            self.get_fields_and_fragment_names(schema, parent_type2, selection_set2);

        // (H) First, collect all conflicts between these two collections of field.
        self.collect_conflicts_between(
            schema,
            &mut conflicts,
            mutually_exclusive,
            &field_map1,
            &field_map2,
            visited_fragments,
        );

        // (I) Then collect conflicts between the first collection of fields and
        // those referenced by each fragment name associated with the second.
        for fragment_name in &fragment_names2 {
            self.collect_conflicts_between_fields_and_fragment(
                schema,
                &mut conflicts,
                &field_map1,
                fragment_name,
                mutually_exclusive,
                visited_fragments,
            );
        }

        // (I) Then collect conflicts between the second collection of fields and
        // those referenced by each fragment name associated with the first.
        for fragment_name in &fragment_names1 {
            self.collect_conflicts_between_fields_and_fragment(
                schema,
                &mut conflicts,
                &field_map2,
                fragment_name,
                mutually_exclusive,
                visited_fragments,
            );
        }

        // (J) Also collect conflicts between any fragment names by the first and
        // fragment names by the second. This compares each item in the first set of
        // names to each item in the second set of names.
        for fragment_name1 in &fragment_names1 {
            for fragment_name2 in &fragment_names2 {
                self.collect_conflicts_between_fragments(
                    schema,
                    &mut conflicts,
                    fragment_name1,
                    fragment_name2,
                    mutually_exclusive,
                    visited_fragments,
                );
            }
        }

        conflicts
    }

    fn collect_conflicts_between_fields_and_fragment(
        &mut self,
        schema: &'a SchemaDocument,
        conflicts: &mut Vec<Conflict>,
        field_map: &OrderedMap<&'a str, Vec<AstAndDef<'a>>>,
        fragment_name: &str,
        mutually_exclusive: bool,
        visited_fragments: &mut Vec<&'a str>,
    ) {
        let fragment = match self.named_fragments.get(fragment_name) {
            Some(f) => f,
            None => return,
        };

        let (field_map2, fragment_names2) =
            self.get_referenced_fields_and_fragment_names(schema, fragment);

        if fragment_names2.contains(&fragment_name) {
            return;
        }

        self.collect_conflicts_between(
            schema,
            conflicts,
            mutually_exclusive,
            field_map,
            &field_map2,
            visited_fragments,
        );

        for fragment_name2 in &fragment_names2 {
            if visited_fragments.contains(fragment_name2) {
                return;
            }

            visited_fragments.push(fragment_name2);

            self.collect_conflicts_between_fields_and_fragment(
                schema,
                conflicts,
                field_map,
                fragment_name2,
                mutually_exclusive,
                visited_fragments,
            );
        }
    }

    // Collect all conflicts found between two fragments, including via spreading in
    // any nested fragments.
    fn collect_conflicts_between_fragments(
        &mut self,
        schema: &'a SchemaDocument,
        conflicts: &mut Vec<Conflict>,
        fragment_name1: &'a str,
        fragment_name2: &'a str,
        mutually_exclusive: bool,
        visited_fragments: &mut Vec<&'a str>,
    ) {
        // No need to compare a fragment to itself.
        if fragment_name1.eq(fragment_name2) {
            return;
        }

        // Memoize so two fragments are not compared for conflicts more than once.
        if self
            .compared_fragments
            .contains(fragment_name1, fragment_name2, mutually_exclusive)
        {
            return;
        }

        self.compared_fragments
            .insert(fragment_name1, fragment_name2, mutually_exclusive);

        let fragment1 = match self.named_fragments.get(fragment_name1) {
            Some(f) => f,
            None => return,
        };

        let fragment2 = match self.named_fragments.get(fragment_name2) {
            Some(f) => f,
            None => return,
        };

        let (field_map1, fragment_names1) =
            self.get_referenced_fields_and_fragment_names(schema, fragment1);
        let (field_map2, fragment_names2) =
            self.get_referenced_fields_and_fragment_names(schema, fragment2);

        // (F) First, collect all conflicts between these two collections of fields
        // (not including any nested fragments).
        self.collect_conflicts_between(
            schema,
            conflicts,
            mutually_exclusive,
            &field_map1,
            &field_map2,
            visited_fragments,
        );

        // (G) Then collect conflicts between the first fragment and any nested
        // fragments spread in the second fragment.
        for fragment_name2 in &fragment_names2 {
            self.collect_conflicts_between_fragments(
                schema,
                conflicts,
                fragment_name1,
                fragment_name2,
                mutually_exclusive,
                visited_fragments,
            );
        }

        // (G) Then collect conflicts between the second fragment and any nested
        // fragments spread in the first fragment.
        for fragment_name1 in &fragment_names1 {
            self.collect_conflicts_between_fragments(
                schema,
                conflicts,
                fragment_name1,
                fragment_name2,
                mutually_exclusive,
                visited_fragments,
            );
        }
    }

    // Given a reference to a fragment, return the represented collection of fields
    // as well as a list of nested fragment names referenced via fragment spreads.
    fn get_referenced_fields_and_fragment_names(
        &self,
        schema: &'a SchemaDocument,
        fragment: &'a FragmentDefinition,
    ) -> (OrderedMap<&'a str, Vec<AstAndDef<'a>>>, Vec<&'a str>) {
        let TypeCondition::On(type_condition) = &fragment.type_condition;
        let fragment_type = schema.type_by_name(type_condition);

        self.get_fields_and_fragment_names(schema, fragment_type, &fragment.selection_set)
    }

    // Collect all Conflicts between two collections of fields. This is similar to,
    // but different from the `collectConflictsWithin` function above. This check
    // assumes that `collectConflictsWithin` has already been called on each
    // provided collection of fields. This is true because this validator traverses
    // each individual selection set.
    fn collect_conflicts_between(
        &mut self,
        schema: &'a SchemaDocument,
        conflicts: &mut Vec<Conflict>,
        mutually_exclusive: bool,
        field_map1: &OrderedMap<&'a str, Vec<AstAndDef<'a>>>,
        field_map2: &OrderedMap<&'a str, Vec<AstAndDef<'a>>>,
        visited_fragments: &mut Vec<&'a str>,
    ) {
        // A field map is a keyed collection, where each key represents a response
        // name and the value at that key is a list of all fields which provide that
        // response name. For any response name which appears in both provided field
        // maps, each field from the first field map must be compared to every field
        // in the second field map to find potential conflicts.
        for (response_name, fields1) in field_map1.iter() {
            if let Some(fields2) = field_map2.get(response_name) {
                for field1 in fields1 {
                    for field2 in fields2 {
                        if let Some(conflict) = self.find_conflict(
                            schema,
                            response_name,
                            field1,
                            field2,
                            mutually_exclusive,
                            visited_fragments,
                        ) {
                            conflicts.push(conflict);
                        }
                    }
                }
            }
        }
    }

    // Given a selection set, return the collection of fields (a mapping of response
    // name to field nodes and definitions) as well as a list of fragment names
    // referenced via fragment spreads.
    fn get_fields_and_fragment_names(
        &self,
        schema: &'a SchemaDocument,
        parent_type: Option<&'a TypeDefinition>,
        selection_set: &'a SelectionSet,
    ) -> (OrderedMap<&'a str, Vec<AstAndDef<'a>>>, Vec<&'a str>) {
        let mut ast_and_defs = OrderedMap::new();
        let mut fragment_names = Vec::new();

        Self::collect_fields_and_fragment_names(
            schema,
            parent_type,
            selection_set,
            &mut ast_and_defs,
            &mut fragment_names,
        );

        (ast_and_defs, fragment_names)
    }

    fn collect_fields_and_fragment_names(
        schema: &'a SchemaDocument,
        parent_type: Option<&'a TypeDefinition>,
        selection_set: &'a SelectionSet,
        ast_and_defs: &mut OrderedMap<&'a str, Vec<AstAndDef<'a>>>,
        fragment_names: &mut Vec<&'a str>,
    ) {
        for selection in &selection_set.items {
            match selection {
                Selection::Field(field) => {
                    let field_name = &field.name;
                    let field_def = parent_type.and_then(|t| t.field_by_name(field_name));
                    let out_field_name = field.alias.as_ref().unwrap_or(field_name).as_str();

                    if !ast_and_defs.contains_key(out_field_name) {
                        ast_and_defs.insert(out_field_name, Vec::new());
                    }

                    ast_and_defs
                        .get_mut(out_field_name)
                        .unwrap()
                        .push(AstAndDef(parent_type, field, field_def));
                }
                Selection::FragmentSpread(fragment_spread) => {
                    if !fragment_names
                        .iter()
                        .any(|n| (*n).eq(&fragment_spread.fragment_name))
                    {
                        fragment_names.push(&fragment_spread.fragment_name);
                    }
                }
                Selection::InlineFragment(inline_fragment) => {
                    let fragment_type = inline_fragment
                        .type_condition
                        .as_ref()
                        .and_then(|type_condition| {
                            let TypeCondition::On(type_condition) = type_condition;

                            schema.type_by_name(type_condition)
                        })
                        .or(parent_type);

                    Self::collect_fields_and_fragment_names(
                        schema,
                        fragment_type,
                        &inline_fragment.selection_set,
                        ast_and_defs,
                        fragment_names,
                    )
                }
            }
        }
    }
}

impl<'a> OperationVisitor<'a, ValidationErrorContext> for OverlappingFieldsCanBeMerged<'a> {
    fn enter_document(
        &mut self,
        _visitor_context: &mut OperationVisitorContext,
        _: &mut ValidationErrorContext,
        document: &'a Document,
    ) {
        for definition in &document.definitions {
            if let Definition::Fragment(fragment) = definition {
                self.named_fragments.insert(&fragment.name, fragment);
            }
        }
    }

    fn enter_selection_set(
        &mut self,
        visitor_context: &mut OperationVisitorContext<'a>,
        user_context: &mut ValidationErrorContext,
        selection_set: &'a SelectionSet,
    ) {
        let parent_type = visitor_context.current_parent_type();
        let schema = visitor_context.schema;
        let mut visited_fragments = Vec::new();
        let found_conflicts = self.find_conflicts_within_selection_set(
            schema,
            parent_type,
            selection_set,
            &mut visited_fragments,
        );

        for Conflict(ConflictReason(reason_name, reason_msg), mut p1, p2) in found_conflicts {
            p1.extend(p2);

            user_context.report_error(ValidationError {
                error_code: self.error_code(),
                message: error_message(&reason_name, &reason_msg),
                locations: p1,
            });
        }
    }
}

fn error_message(reason_name: &str, reason: &ConflictReasonMessage) -> String {
    let suffix = "Use different aliases on the fields to fetch both if this was intentional.";

    format!(
        r#"Fields "{}" conflict because {}. {}"#,
        reason_name,
        format_reason(reason),
        suffix
    )
}

fn format_reason(reason: &ConflictReasonMessage) -> String {
    match *reason {
        ConflictReasonMessage::Message(ref name) => name.clone(),
        ConflictReasonMessage::Nested(ref nested) => nested
            .iter()
            .map(|ConflictReason(name, subreason)| {
                format!(
                    r#"subfields "{}" conflict because {}"#,
                    name,
                    format_reason(subreason)
                )
            })
            .collect::<Vec<_>>()
            .join(" and "),
    }
}

impl<'o> ValidationRule for OverlappingFieldsCanBeMerged<'o> {
    fn error_code<'a>(&self) -> &'a str {
        "OverlappingFieldsCanBeMerged"
    }

    fn validate(
        &self,
        ctx: &mut OperationVisitorContext,
        error_collector: &mut ValidationErrorContext,
    ) {
        visit_document(
            &mut OverlappingFieldsCanBeMerged::new(),
            ctx.operation,
            ctx,
            error_collector,
        );
    }
}

#[test]
fn unique_fields() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "fragment uniqueFields on Dog {
          name
          nickname
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn identical_fields() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "fragment mergeIdenticalFields on Dog {
          name
          name
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn identical_fields_with_identical_variables() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        r#"fragment mergeIdenticalFieldsWithIdenticalArgs on Dog {
          doesKnowCommand(dogCommand: $dogCommand)
          doesKnowCommand(dogCommand: $dogCommand)
        }"#,
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn identical_fields_with_different_variables() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        r#"fragment mergeIdenticalFieldsWithIdenticalArgs on Dog {
          doesKnowCommand(dogCommand: $catCommand)
          doesKnowCommand(dogCommand: $dogCommand)
        }"#,
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages, vec!["Fields \"doesKnowCommand\" conflict because they have differing arguments. Use different aliases on the fields to fetch both if this was intentional."]);
}

#[test]
fn identical_fields_and_identical_args() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "fragment mergeIdenticalFieldsWithIdenticalArgs on Dog {
          doesKnowCommand(dogCommand: SIT)
          doesKnowCommand(dogCommand: SIT)
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn identical_fields_and_identical_directives() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "fragment mergeSameFieldsWithSameDirectives on Dog {
          name @include(if: true)
          name @include(if: true)
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn different_args_different_aliases() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "fragment differentArgsWithDifferentAliases on Dog {
          knowsSit: doesKnowCommand(dogCommand: SIT)
          knowsDown: doesKnowCommand(dogCommand: DOWN)
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn different_directives_different_aliases() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "fragment differentDirectivesWithDifferentAliases on Dog {
          nameIfTrue: name @include(if: true)
          nameIfFalse: name @include(if: false)
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn different_skip_include_directives() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "fragment differentDirectivesWithDifferentAliases on Dog {
          name @include(if: true)
          name @include(if: false)
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    assert_eq!(get_messages(&errors).len(), 0);
}

#[test]
fn same_alias_different_field_target() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "fragment sameAliasesWithDifferentFieldTargets on Dog {
          fido: name
          fido: nickname
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages, vec!["Fields \"fido\" conflict because \"name\" and \"nickname\" are different fields. Use different aliases on the fields to fetch both if this was intentional."]);
}

#[test]
fn same_alias_non_overlapping_field_target() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "fragment sameAliasesWithDifferentFieldTargets on Pet {
          ... on Dog {
            name
          }
          ... on Cat {
            name: nickname
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn alias_masking_direct_access() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "fragment aliasMaskingDirectFieldAccess on Dog {
          name: nickname
          name
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages, vec!["Fields \"name\" conflict because \"nickname\" and \"name\" are different fields. Use different aliases on the fields to fetch both if this was intentional."]);
}

#[test]
fn different_args_second_adds() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "fragment conflictingArgs on Dog {
          doesKnowCommand
          doesKnowCommand(dogCommand: HEEL)
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages, vec!["Fields \"doesKnowCommand\" conflict because they have differing arguments. Use different aliases on the fields to fetch both if this was intentional."]);
}

#[test]
fn different_args_declared_on_first() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "fragment conflictingArgs on Dog {
          doesKnowCommand(dogCommand: SIT)
          doesKnowCommand
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages, vec!["Fields \"doesKnowCommand\" conflict because they have differing arguments. Use different aliases on the fields to fetch both if this was intentional."]);
}

#[test]
fn different_arg_values() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "fragment conflictingArgs on Dog {
          doesKnowCommand(dogCommand: SIT)
          doesKnowCommand(dogCommand: HEEL)
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages, vec!["Fields \"doesKnowCommand\" conflict because they have differing arguments. Use different aliases on the fields to fetch both if this was intentional."]);
}

#[test]
fn conflicting_arg_names() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "fragment conflictingArgs on Dog {
          isAtLocation(x: 0)
          isAtLocation(y: 0)
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages, vec!["Fields \"isAtLocation\" conflict because they have differing arguments. Use different aliases on the fields to fetch both if this was intentional."]);
}

#[test]
fn allow_different_args_when_possible_with_different_args() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "fragment conflictingArgs on Pet {
          ... on Dog {
            name(surname: true)
          }
          ... on Cat {
            name
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn conflict_in_fragment_spread() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "query {
          ...A
          ...B
        }
        fragment A on Type {
          x: a
        }
        fragment B on Type {
          x: b
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages, vec!["Fields \"x\" conflict because \"a\" and \"b\" are different fields. Use different aliases on the fields to fetch both if this was intentional."]);
}

#[test]
fn deep_conflict() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "{
          field {
            x: a
          }
          field {
            x: b
          }
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages, vec!["Fields \"field\" conflict because subfields \"x\" conflict because \"a\" and \"b\" are different fields. Use different aliases on the fields to fetch both if this was intentional."]);
}

#[test]
fn report_each_conflict_once() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "{
          f1 {
            ...A
            ...B
          }
          f2 {
            ...B
            ...A
          }
          f3 {
            ...A
            ...B
            x: c
          }
        }
        fragment A on Type {
          x: a
        }
        fragment B on Type {
          x: b
        }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 3);
    assert_eq!(messages, vec![
      "Fields \"x\" conflict because \"a\" and \"b\" are different fields. Use different aliases on the fields to fetch both if this was intentional.",
      "Fields \"x\" conflict because \"c\" and \"a\" are different fields. Use different aliases on the fields to fetch both if this was intentional.",
      "Fields \"x\" conflict because \"c\" and \"b\" are different fields. Use different aliases on the fields to fetch both if this was intentional."
    ]);
}

#[cfg(test)]
pub static OVERLAPPING_RULE_TEST_SCHEMA: &str = "
interface SomeBox {
  deepBox: SomeBox
  unrelatedField: String
}
type StringBox implements SomeBox {
  scalar: String
  deepBox: StringBox
  unrelatedField: String
  listStringBox: [StringBox]
  stringBox: StringBox
  intBox: IntBox
}
type IntBox implements SomeBox {
  scalar: Int
  deepBox: IntBox
  unrelatedField: String
  listStringBox: [StringBox]
  stringBox: StringBox
  intBox: IntBox
}
interface NonNullStringBox1 {
  scalar: String!
}
type NonNullStringBox1Impl implements SomeBox & NonNullStringBox1 {
  scalar: String!
  unrelatedField: String
  deepBox: SomeBox
}
interface NonNullStringBox2 {
  scalar: String!
}
type NonNullStringBox2Impl implements SomeBox & NonNullStringBox2 {
  scalar: String!
  unrelatedField: String
  deepBox: SomeBox
}
type Connection {
  edges: [Edge]
}
type Edge {
  node: Node
}
type Node {
  id: ID
  name: String
}
type Query {
  someBox: SomeBox
  connection: Connection
}";

#[test]
fn conflicting_return_types_which_potentially_overlap() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "{
          someBox {
            ...on IntBox {
              scalar
            }
            ...on NonNullStringBox1 {
              scalar
            }
          }
        }",
        OVERLAPPING_RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages, vec![
      "Fields \"scalar\" conflict because they return conflicting types \"Int\" and \"String!\". Use different aliases on the fields to fetch both if this was intentional."
    ]);
}

#[test]
fn compatible_return_shapes_on_different_return_types() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "{
          someBox {
            ... on SomeBox {
              deepBox {
                unrelatedField
              }
            }
            ... on StringBox {
              deepBox {
                unrelatedField
              }
            }
          }
        }",
        OVERLAPPING_RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn disallows_differing_return_types_despite_no_overlap() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "{
          someBox {
            ... on IntBox {
              scalar
            }
            ... on StringBox {
              scalar
            }
          }
        }",
        OVERLAPPING_RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages, vec![
      "Fields \"scalar\" conflict because they return conflicting types \"Int\" and \"String\". Use different aliases on the fields to fetch both if this was intentional."
    ]);
}

#[test]
fn reports_correctly_when_a_non_exclusive_follows_an_exclusive() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "{
          someBox {
            ... on IntBox {
              deepBox {
                ...X
              }
            }
          }
          someBox {
            ... on StringBox {
              deepBox {
                ...Y
              }
            }
          }
          memoed: someBox {
            ... on IntBox {
              deepBox {
                ...X
              }
            }
          }
          memoed: someBox {
            ... on StringBox {
              deepBox {
                ...Y
              }
            }
          }
          other: someBox {
            ...X
          }
          other: someBox {
            ...Y
          }
        }
        fragment X on SomeBox {
          scalar
        }
        fragment Y on SomeBox {
          scalar: unrelatedField
        }",
        OVERLAPPING_RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages, vec![
      "Fields \"other\" conflict because subfields \"scalar\" conflict because \"scalar\" and \"unrelatedField\" are different fields. Use different aliases on the fields to fetch both if this was intentional."
    ]);
}

#[test]
fn disallows_differing_return_type_nullability_despite_no_overlap() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "{
          someBox {
            ... on NonNullStringBox1 {
              scalar
            }
            ... on StringBox {
              scalar
            }
          }
        }",
        OVERLAPPING_RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages, vec![
      "Fields \"scalar\" conflict because they return conflicting types \"String!\" and \"String\". Use different aliases on the fields to fetch both if this was intentional."
    ]);
}

#[test]
fn disallows_differing_return_type_list_despite_no_overlap() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "{
          someBox {
            ... on IntBox {
              box: listStringBox {
                scalar
              }
            }
            ... on StringBox {
              box: stringBox {
                scalar
              }
            }
          }
        }",
        OVERLAPPING_RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages, vec![
      "Fields \"box\" conflict because they return conflicting types \"[StringBox]\" and \"StringBox\". Use different aliases on the fields to fetch both if this was intentional."
    ]);

    let errors = test_operation_with_schema(
        "{
            someBox {
              ... on IntBox {
                box: stringBox {
                  scalar
                }
              }
              ... on StringBox {
                box: listStringBox {
                  scalar
                }
              }
            }
          }",
        OVERLAPPING_RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages, vec![
      "Fields \"box\" conflict because they return conflicting types \"StringBox\" and \"[StringBox]\". Use different aliases on the fields to fetch both if this was intentional."
    ]);
}

#[test]
fn disallows_differing_subfields() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "{
          someBox {
            ... on IntBox {
              box: stringBox {
                val: scalar
                val: unrelatedField
              }
            }
            ... on StringBox {
              box: stringBox {
                val: scalar
              }
            }
          }
        }",
        OVERLAPPING_RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages, vec![
      "Fields \"val\" conflict because \"scalar\" and \"unrelatedField\" are different fields. Use different aliases on the fields to fetch both if this was intentional."
    ]);
}

#[test]
fn disallows_differing_deep_return_types_despite_no_overlap() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "{
          someBox {
            ... on IntBox {
              box: stringBox {
                scalar
              }
            }
            ... on StringBox {
              box: intBox {
                scalar
              }
            }
          }
        }",
        OVERLAPPING_RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages, vec![
      "Fields \"box\" conflict because subfields \"scalar\" conflict because they return conflicting types \"String\" and \"Int\". Use different aliases on the fields to fetch both if this was intentional."
    ]);
}

#[test]
fn allows_non_conflicting_overlapping_types() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "{
          someBox {
            ... on IntBox {
              scalar: unrelatedField
            }
            ... on StringBox {
              scalar
            }
          }
        }",
        OVERLAPPING_RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn same_wrapped_scalar_return_types() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "{
          someBox {
            ...on NonNullStringBox1 {
              scalar
            }
            ...on NonNullStringBox2 {
              scalar
            }
          }
        }",
        OVERLAPPING_RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn allows_inline_fragments_without_type_condition() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "{
          a
          ... {
            a
          }
        }",
        OVERLAPPING_RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn compares_deep_types_including_list() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "{
          connection {
            ...edgeID
            edges {
              node {
                id: name
              }
            }
          }
        }
        fragment edgeID on Connection {
          edges {
            node {
              id
            }
          }
        }",
        OVERLAPPING_RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages, vec![
      "Fields \"edges\" conflict because subfields \"node\" conflict because subfields \"id\" conflict because \"name\" and \"id\" are different fields. Use different aliases on the fields to fetch both if this was intentional."
    ]);
}

#[test]
fn ignores_unknown_types() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "{
          someBox {
            ...on UnknownType {
              scalar
            }
            ...on NonNullStringBox2 {
              scalar
            }
          }
        }",
        OVERLAPPING_RULE_TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn does_not_infinite_loop_on_recursive_fragment() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "fragment fragA on Human { name, relatives { name, ...fragA } }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn does_not_infinite_loop_on_immediately_recursive_fragment() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "fragment fragA on Human { name, ...fragA }",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn does_not_infinite_loop_on_transitively_recursive_fragment() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "
        fragment fragA on Human { name, ...fragB }
        fragment fragB on Human { name, ...fragC }
        fragment fragC on Human { name, ...fragA }
      ",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 0);
}

#[test]
fn finds_invalid_case_even_with_immediately_recursive_fragment() {
    use crate::validation::test_utils::*;

    let mut plan = create_plan_from_rule(Box::new(OverlappingFieldsCanBeMerged::new()));
    let errors = test_operation_with_schema(
        "
        fragment sameAliasesWithDifferentFieldTargets on Dog {
          ...sameAliasesWithDifferentFieldTargets
          fido: name
          fido: nickname
        }
      ",
        TEST_SCHEMA,
        &mut plan,
    );

    let messages = get_messages(&errors);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages, vec![
      "Fields \"fido\" conflict because \"name\" and \"nickname\" are different fields. Use different aliases on the fields to fetch both if this was intentional."
    ]);
}
