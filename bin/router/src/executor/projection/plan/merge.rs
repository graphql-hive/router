use crate::executor::introspection::schema::SchemaMetadata;
use crate::executor::utils::consts::TYPENAME_FIELD_NAME;
use bumpalo::collections::Vec as BumpVec;
use bumpalo::Bump;
use indexmap::IndexMap;

use super::ir::{self, Field, TypeSet, Value};

/// Fields sharing a response key, in first-seen order.
/// Each group holds the variants that still have distinct parent scopes.
type ResponseKeyGroups<'arena> = IndexMap<&'arena str, BumpVec<'arena, Field<'arena>>>;

pub(super) struct Merger<'arena> {
    schema: &'arena SchemaMetadata,
    arena: &'arena Bump,
}

impl<'arena> Merger<'arena> {
    pub(super) fn new(schema: &'arena SchemaMetadata, arena: &'arena Bump) -> Self {
        Self { schema, arena }
    }

    pub(super) fn merge_fields<'a>(
        collected: BumpVec<'a, Field<'a>>,
        schema: &'a SchemaMetadata,
        parent_type: &'a str,
        arena: &'a Bump,
    ) -> BumpVec<'a, Field<'a>> {
        let merger = Merger::new(schema, arena);
        let mut merged = merger.merge_selection_set(collected, parent_type);
        merger.prune_redundant_scopes(&mut merged, parent_type);
        merged
    }

    fn child_scope(
        &self,
        parent_type: &'arena str,
        scope: Option<TypeSet<'arena>>,
        field_name: &str,
    ) -> &'arena str {
        self.child_parent_type(parent_type, scope, field_name)
            .unwrap_or(parent_type)
    }

    fn merge_selection_set(
        &self,
        mut siblings: BumpVec<'arena, Field<'arena>>,
        parent_type: &'arena str,
    ) -> BumpVec<'arena, Field<'arena>> {
        for field in &mut siblings {
            let Value::Children(sub_fields) =
                std::mem::replace(&mut field.value, Value::Passthrough)
            else {
                continue;
            };

            let child_type = self.child_scope(parent_type, field.parent_scope, field.field_name);
            field.value = Value::Children(self.merge_selection_set(sub_fields, child_type));
        }

        let mut key_groups = ResponseKeyGroups::new();
        for next in siblings {
            let variants = key_groups
                .entry(next.response_key)
                .or_insert_with(|| BumpVec::new_in(self.arena));
            match variants
                .iter_mut()
                .find(|kept| kept.parent_scope == next.parent_scope)
            {
                Some(kept) => self.merge(kept, next, parent_type),
                None => variants.push(next),
            }
        }

        self.merge_groups(key_groups, parent_type)
    }

    /// Merges `next` into `kept`. Both share one response key and one parent scope.
    ///
    /// Conditions are OR-ed so neither branch is dropped.
    fn merge(&self, kept: &mut Field<'arena>, next: Field<'arena>, parent_type: &'arena str) {
        let condition = std::mem::replace(&mut kept.condition, ir::Condition::Always);
        kept.condition = condition.or(next.condition, self.arena);

        // Children only merge with children.
        // `Null` never appears here: it is only introduced later by rewriting,
        // and merging always combines selections, so a mismatched shape keeps the kept value.
        let child_type = self.child_scope(parent_type, kept.parent_scope, kept.field_name);
        let (Value::Children(kept_children), Value::Children(next_children)) =
            (&mut kept.value, next.value)
        else {
            return;
        };
        let mut joined =
            BumpVec::with_capacity_in(kept_children.len() + next_children.len(), self.arena);
        joined.extend(kept_children.drain(..));
        joined.extend(next_children);
        *kept_children = self.merge_selection_set(joined, child_type);
    }

    /// Collapses each response key's variants into the final selections.
    ///
    /// A response key may be selected for multiple parent types with
    /// different nested selections:
    ///
    /// `... on Article { meta { title } }`
    /// `... on Video { meta { wordCount } }`
    ///
    /// Overlapping variants are specialized first (see below), so each
    /// concrete type ends up with its own exact variant and nested
    /// selections never "bleed" into one another.
    /// At runtime the `meta` plan then matches exactly one variant,
    /// based on the concrete `__typename`.
    fn merge_groups(
        &self,
        key_groups: ResponseKeyGroups<'arena>,
        parent_type: &'arena str,
    ) -> BumpVec<'arena, Field<'arena>> {
        let mut object_types: Option<BumpVec<'arena, &'arena str>> = None;
        let mut merged = BumpVec::new_in(self.arena);
        for (_, variants) in key_groups {
            // One variant, or scopes that are all single concrete types, can
            // never match the same object twice.
            let exclusive = variants.len() == 1
                || variants
                    .iter()
                    .all(|field| field.parent_scope.is_some_and(|scope| scope.len() == 1));

            if exclusive {
                merged.extend(variants);
                continue;
            }

            let object_types = object_types.get_or_insert_with(|| {
                let mut names = BumpVec::new_in(self.arena);
                if let Some(possible) = self.schema.get_possible_types(parent_type) {
                    names.extend(
                        possible
                            .iter()
                            .map(String::as_str)
                            .filter(|obj_type| self.schema.is_object_type(obj_type)),
                    );
                }

                if self.schema.is_object_type(parent_type) {
                    names.push(parent_type);
                }

                names.sort_unstable();
                names.dedup();
                names
            });

            self.specialize_variants(variants, parent_type, object_types, &mut merged);
        }

        merged
    }

    /// Clones an overlapping group into one exact variant per concrete type.
    fn specialize_variants(
        &self,
        variants: BumpVec<'arena, Field<'arena>>,
        parent_type: &'arena str,
        object_types: &[&'arena str],
        merged: &mut BumpVec<'arena, Field<'arena>>,
    ) {
        for obj_type in object_types.iter() {
            let mut merged_variant: Option<Field<'arena>> = None;
            for field in &variants {
                if !field
                    .parent_scope
                    .is_none_or(|scope| scope.matches(obj_type))
                {
                    continue;
                }

                let mut exact_variant = field.clone();
                exact_variant.parent_scope = Some(TypeSet::exact(self.arena, obj_type));

                match &mut merged_variant {
                    None => merged_variant = Some(exact_variant),
                    Some(kept) => self.merge(kept, exact_variant, parent_type),
                }
            }
            merged.extend(merged_variant);
        }
    }

    /// Removes child scopes that match every possible output of their field.
    ///
    /// A child scope is redundant when it covers exactly all types that can
    /// appear in that position.
    /// For example, if field `item` on parent `A` can only return `Item`,
    /// a child scope `[Item]` always matches.
    /// Every scope costs time at runtime, so dropping these speeds up
    /// response projection.
    /// Runs after merging, when scopes are final.
    fn prune_redundant_scopes(
        &self,
        siblings: &mut BumpVec<'arena, Field<'arena>>,
        parent_type: &'arena str,
    ) {
        for field in siblings.iter_mut() {
            let (scope, field_name) = (field.parent_scope, field.field_name);
            let child_type = self.child_scope(parent_type, scope, field_name);
            let Value::Children(children) = &mut field.value else {
                continue;
            };
            if let Some(possible) = self.possible_output_types(scope, field_name) {
                for child in children.iter_mut() {
                    // Both sides are sorted and deduplicated, so an equal set
                    // means the scope check can never fail.
                    if child.parent_scope == Some(possible) {
                        child.parent_scope = None;
                        continue;
                    }
                    let condition = std::mem::replace(&mut child.condition, ir::Condition::Always);
                    child.condition =
                        condition.without_redundant_parent_scope(&child.parent_scope, self.arena);
                }
            }
            self.prune_redundant_scopes(children, child_type);
        }
    }

    /// Lists every output type a field can return under its scope.
    /// Returns `None` when the scope is unknown or the field is missing.
    fn possible_output_types(
        &self,
        parent_scope: Option<TypeSet<'arena>>,
        field_name: &str,
    ) -> Option<TypeSet<'arena>> {
        let mut outputs = BumpVec::new_in(self.arena);
        outputs.extend(
            parent_scope?
                .iter()
                .filter_map(|member| self.output_on(member, field_name)),
        );

        if outputs.is_empty() {
            None
        } else {
            Some(TypeSet::sorted(outputs))
        }
    }

    fn output_on(&self, on_type: &str, field_name: &str) -> Option<&'arena str> {
        if field_name == TYPENAME_FIELD_NAME {
            return None;
        }

        self.schema
            .type_fields
            .get(on_type)
            .and_then(|fields| fields.get(field_name))
            .map(|info| info.output_type_name.as_str())
    }

    /// Returns the field's output type for the given parent type.
    ///
    /// Members of an abstract scope may declare different output types.
    /// In that case, prefer the type declared on the parent.
    /// If the parent has no declaration, choose deterministically by member name.
    pub(super) fn child_parent_type(
        &self,
        parent_type: &'arena str,
        parent_scope: Option<TypeSet<'arena>>,
        field_name: &str,
    ) -> Option<&'arena str> {
        let Some(scope) = parent_scope else {
            return self.output_on(parent_type, field_name);
        };

        let mut outputs = scope
            .iter()
            .filter_map(|member| self.output_on(member, field_name));

        let first = outputs.next()?;
        let mut min_output = first;
        let mut mixed = false;
        for output in outputs {
            min_output = min_output.min(output);
            mixed |= output != first;
        }

        if !mixed {
            return Some(first);
        }

        self.output_on(parent_type, field_name).or(Some(min_output))
    }
}

#[cfg(test)]
mod tests {
    use crate::executor::introspection::schema::SchemaWithMetadata;
    use crate::executor::projection::plan::ProjectionPlan;
    use crate::query_planner::ast::normalization::normalize_operation;
    use crate::query_planner::utils::parsing::{parse_operation, parse_schema};
    use crate::query_planner::{planner::Planner, state::supergraph_state::SupergraphState};

    #[test]
    fn child_scopes_covering_all_outputs_are_pruned() {
        let schema = parse_schema(
            r#"
            interface Node { id: ID!, item: Item }
            type A implements Node { id: ID!, item: Item }
            type B implements Node { id: ID!, item: Item }
            type Item { id: ID! }
            type Query { node: Node }
            "#,
        );
        let supergraph = SupergraphState::new(&schema);
        let planner = Planner::new_from_supergraph(&schema, Default::default()).unwrap();
        let operation = parse_operation(
            r#"
            query Example {
                node {
                    ... on A { item { id } }
                    ... on B { item { id } }
                }
            }
            "#,
        );
        let normalized = normalize_operation(&supergraph, &operation, None).unwrap();
        let schema_metadata = planner.consumer_schema.schema_metadata();
        let (_, plan) =
            ProjectionPlan::from_operation(normalized.executable_operation(), &schema_metadata);

        let rendered = plan.to_string();
        // The two `item` variants keep their guards (A vs B still differ),
        // but `id` under each one always sees `Item`, so its guard is dropped.
        assert_eq!(rendered.matches("type guard").count(), 2);
        assert!(rendered.contains("id"));
    }

    fn plan(schema: &str, query: &str) -> String {
        let supergraph = parse_schema(schema);
        let planner = Planner::new_from_supergraph(&supergraph, Default::default()).unwrap();
        let schema_metadata = planner.consumer_schema.schema_metadata();

        let operation = parse_operation(query);
        let supergraph_state = SupergraphState::new(&supergraph);
        let normalized =
            normalize_operation(&supergraph_state, &operation, None).expect("failed to normalize");
        let (_, plan) =
            ProjectionPlan::from_operation(normalized.executable_operation(), &schema_metadata);

        plan.to_string()
    }

    const SCHEMA_1: &str = r#"
    schema
        @link(url: "https://specs.apollo.dev/link/v1.0")
        @link(url: "https://specs.apollo.dev/join/v0.3", for: EXECUTION) {
        query: Query
    }

    directive @join__enumValue(graph: join__Graph!) repeatable on ENUM_VALUE

    directive @join__graph(name: String!, url: String!) on ENUM_VALUE

    directive @join__field(
        graph: join__Graph
        requires: join__FieldSet
        provides: join__FieldSet
        type: String
        external: Boolean
        override: String
        usedOverridden: Boolean
    ) repeatable on FIELD_DEFINITION | INPUT_FIELD_DEFINITION

    directive @join__implements(
        graph: join__Graph!
        interface: String!
    ) repeatable on OBJECT | INTERFACE

    directive @join__type(
        graph: join__Graph!
        key: join__FieldSet
        extension: Boolean! = false
        resolvable: Boolean! = true
        isInterfaceObject: Boolean! = false
    ) repeatable on OBJECT | INTERFACE | UNION | ENUM | INPUT_OBJECT | SCALAR

    directive @join__unionMember(
        graph: join__Graph!
        member: String!
    ) repeatable on UNION

    scalar join__FieldSet

    directive @link(
        url: String
        as: String
        for: link__Purpose
        import: [link__Import]
    ) repeatable on SCHEMA

    scalar link__Import

    enum link__Purpose {
        """
        `SECURITY` features provide metadata necessary to securely resolve fields.
        """
        SECURITY

        """
        `EXECUTION` features provide metadata necessary for operation execution.
        """
        EXECUTION
    }

    enum join__Graph {
        LOCAL @join__graph(name: "local", url: "")
    }

    type Query @join__type(graph: LOCAL) {
        search: [SearchResult!]!
        feed: [Content!]!
        concrete: ConcreteType
    }

    type ConcreteType @join__type(graph: LOCAL) {
        id: ID!
        test: String!
        someEnum: SomeEnum
        inner: InnerType
    }

    type InnerType @join__type(graph: LOCAL) {
        inner: Boolean
    }

    type Article implements Content
        @join__type(graph: LOCAL)
        @join__implements(graph: LOCAL, interface: "Content") {
        id: ID!
        meta: Meta!
        headline: String!
    }

    type Video implements Content
        @join__type(graph: LOCAL)
        @join__implements(graph: LOCAL, interface: "Content") {
        id: ID!
        meta: Meta!
        duration: Int!
    }

    type Photo implements Content
        @join__type(graph: LOCAL)
        @join__implements(graph: LOCAL, interface: "Content") {
        id: ID!
        meta: Meta!
    }

    type Meta @join__type(graph: LOCAL) {
        title: String
        wordCount: Int!
        arch: String!
    }

    interface Content @join__type(graph: LOCAL) {
        id: ID!
        meta: Meta!
    }

    union SearchResult
        @join__type(graph: LOCAL)
        @join__unionMember(graph: LOCAL, member: "Article")
        @join__unionMember(graph: LOCAL, member: "Video") =
        | Article
        | Video

    enum SomeEnum @join__type(graph: LOCAL) {
        TEST @join__enumValue(graph: LOCAL)
    }
    "#;

    #[test]
    fn simple_concrete_mixed_fields_defs() {
        insta::assert_snapshot!(plan(SCHEMA_1,
            r#"{
                concrete {
                    id
                    test
                    someEnum
                    inner {
                        inner
                    }
                }
            }"#
        ), @r###"
        concrete: {
          selections:
            id: {
            }
            test: {
            }
            someEnum: {
              conditions: EnumValues(TEST)
            }
            inner: {
              selections:
                inner: {
                }
            }
        }
        "###);
    }

    #[test]
    fn inline_fragment_on_concrete_type_with_dups() {
        insta::assert_snapshot!(plan(SCHEMA_1,
            r#"{
                concrete {
                  ... on ConcreteType {
                    id
                    inner {
                        inner
                    }
                  }
                  test
                  someEnum
                  inner {
                      inner
                  }
                }
            }"#
        ), @r###"
        concrete: {
          selections:
            id: {
            }
            inner: {
              selections:
                inner: {
                }
            }
            test: {
            }
            someEnum: {
              conditions: EnumValues(TEST)
            }
        }
        "###);
    }

    /// #1166: union members select the shared key `meta` with disjoint
    /// sub-fields.
    #[test]
    fn union_disjoint_fragments_kept_per_type() {
        insta::assert_snapshot!(plan(SCHEMA_1,
            r#"{
                search {
                    __typename
                    ... on Article { meta { title wordCount } }
                    ... on Video { meta { title arch } }
                }
            }"#
        ), @r###"
        search: {
          conditions: FieldType(OneOf(Article, SearchResult, Video))
          selections:
            __typename: {
            }
            meta: {
              type guard: Exact(Article)
              selections:
                title: {
                }
                wordCount: {
                }
            }
            meta: {
              type guard: Exact(Video)
              selections:
                title: {
                }
                arch: {
                }
            }
        }
        "###);
    }

    /// An "unguarded" selection (no concrete type) of `meta` (on the `Content` interface) overlaps a
    /// guarded one (`... on Article`).
    ///
    /// The overlap is split per concrete type:
    /// `Article` -> merges both
    /// `Video`/`Photo` keep only the shared field
    #[test]
    fn interface_overlapping_guard_splits_per_type() {
        insta::assert_snapshot!(plan(SCHEMA_1,
            r#"{
                feed {
                    meta { title }
                    ... on Article { meta { wordCount } }
                }
            }"#
        ), @r###"
        feed: {
          conditions: FieldType(OneOf(Article, Content, Photo, Video))
          selections:
            meta: {
              type guard: Exact(Article)
              selections:
                title: {
                }
                wordCount: {
                }
            }
            meta: {
              type guard: Exact(Photo)
              selections:
                title: {
                }
            }
            meta: {
              type guard: Exact(Video)
              selections:
                title: {
                }
            }
        }
        "###);
    }

    /// Same key, same guard, same selection from two fragments collapses back
    /// into a single plan (no duplications).
    ///
    /// DOTAN: Added this because I was concered that doing split/merge of the plans based of type,
    /// might end up with duplicate projections, now that it's no longer a single one.
    #[test]
    fn same_guard_fragments_merge_into_one() {
        insta::assert_snapshot!(plan(SCHEMA_1,
            r#"{
                search {
                    ... on Article { meta { title } }
                    ... on Article { meta { title } }
                }
            }"#
        ), @r###"
        search: {
          conditions: FieldType(OneOf(Article, SearchResult, Video))
          selections:
            meta: {
              type guard: Exact(Article)
              selections:
                title: {
                }
            }
        }
        "###);
    }

    const SCHEMA_2: &str = r#"
    schema
      @link(url: "https://specs.apollo.dev/link/v1.0")
      @link(url: "https://specs.apollo.dev/join/v0.3", for: EXECUTION) {
      query: Query
    }

    directive @join__enumValue(graph: join__Graph!) repeatable on ENUM_VALUE

    directive @join__graph(name: String!, url: String!) on ENUM_VALUE

    directive @join__field(
      graph: join__Graph
      requires: join__FieldSet
      provides: join__FieldSet
      type: String
      external: Boolean
      override: String
      usedOverridden: Boolean
    ) repeatable on FIELD_DEFINITION | INPUT_FIELD_DEFINITION

    directive @join__implements(
      graph: join__Graph!
      interface: String!
    ) repeatable on OBJECT | INTERFACE

    directive @join__type(
      graph: join__Graph!
      key: join__FieldSet
      extension: Boolean! = false
      resolvable: Boolean! = true
      isInterfaceObject: Boolean! = false
    ) repeatable on OBJECT | INTERFACE | UNION | ENUM | INPUT_OBJECT | SCALAR

    directive @join__unionMember(
      graph: join__Graph!
      member: String!
    ) repeatable on UNION

    scalar join__FieldSet

    directive @link(
      url: String
      as: String
      for: link__Purpose
      import: [link__Import]
    ) repeatable on SCHEMA

    scalar link__Import

    enum link__Purpose {
      """
      `SECURITY` features provide metadata necessary to securely resolve fields.
      """
      SECURITY

      """
      `EXECUTION` features provide metadata necessary for operation execution.
      """
      EXECUTION
    }

    enum join__Graph {
      LOCAL @join__graph(name: "local", url: "")
    }

    type Dog implements Pet & Animal & Node
      @join__type(graph: LOCAL)
      @join__implements(graph: LOCAL, interface: "Pet")
      @join__implements(graph: LOCAL, interface: "Animal")
      @join__implements(graph: LOCAL, interface: "Node") {
      id: ID!
      name: String!
      bestFriend: Animal
      weight(unit: WeightUnit = KG): Float
      nickname: String
      tags: [String!]
    }

    type Cat implements Pet & Animal & Node
      @join__type(graph: LOCAL)
      @join__implements(graph: LOCAL, interface: "Pet")
      @join__implements(graph: LOCAL, interface: "Animal")
      @join__implements(graph: LOCAL, interface: "Node") {
      id: ID!
      name: String!
      bestFriend: Cat
      weight(unit: WeightUnit = KG): Float
      age: Int
      tags: [String!]
    }

    type Robot implements Node
      @join__type(graph: LOCAL)
      @join__implements(graph: LOCAL, interface: "Node") {
      id: ID!
      model: String!
      weight(unit: WeightUnit = KG): Float
    }

    type Owner implements Node
      @join__type(graph: LOCAL)
      @join__implements(graph: LOCAL, interface: "Node") {
      id: ID!
      name: String!
      pets: [Pet!]!
      primaryPet: Pet
    }

    type Query @join__type(graph: LOCAL) {
      pet: Pet
      animal: Animal
      node: Node
      search: SearchResult
      searchMany: [SearchResult!]!
      pets: [Pet!]!
      owner: Owner
    }

    interface Node @join__type(graph: LOCAL) {
      id: ID!
    }

    interface Animal implements Node
      @join__type(graph: LOCAL)
      @join__implements(graph: LOCAL, interface: "Node") {
      id: ID!
      name: String!
    }

    interface Pet implements Animal & Node
      @join__type(graph: LOCAL)
      @join__implements(graph: LOCAL, interface: "Animal")
      @join__implements(graph: LOCAL, interface: "Node") {
      id: ID!
      name: String!
      bestFriend: Animal
      weight(unit: WeightUnit = KG): Float
    }

    union SearchResult
      @join__type(graph: LOCAL)
      @join__unionMember(graph: LOCAL, member: "Dog")
      @join__unionMember(graph: LOCAL, member: "Cat")
      @join__unionMember(graph: LOCAL, member: "Robot") =
      | Dog
      | Cat
      | Robot

    enum WeightUnit @join__type(graph: LOCAL) {
      KG @join__enumValue(graph: LOCAL)
      LB @join__enumValue(graph: LOCAL)
      G @join__enumValue(graph: LOCAL)
    }
    "#;

    /// When the fragment's id merges into the existing id entry, the key keeps the position of its
    /// first occurrence — it does not move to after nickname.
    #[test]
    fn field_order_when_grouping_with_fragments() {
        let plan = plan(
            SCHEMA_2,
            r#"
        query FieldOrder {
          animal {
            name
            id
            ... on Dog {
              id
              nickname
            }
          }
        }"#,
        );

        insta::assert_snapshot!(plan, @r###"
        animal: {
          conditions: FieldType(OneOf(Animal, Cat, Dog, Pet))
          selections:
            name: {
            }
            id: {
              type guard: Exact(Cat)
            }
            id: {
              type guard: Exact(Dog)
            }
            nickname: {
              type guard: Exact(Dog)
            }
        }
        "###);
    }

    #[test]
    fn multiple_aliases() {
        let plan = plan(
            SCHEMA_2,
            r#"
            query AliasFanout {
              pet {
                ... on Dog {
                  kg: weight(unit: KG)
                  lb: weight(unit: LB)
                  g:  weight(unit: G)
                }
                ... on Cat {
                  kg: weight(unit: KG)
                }
              }
            }"#,
        );

        insta::assert_snapshot!(plan, @r###"
        pet: {
          conditions: FieldType(OneOf(Cat, Dog, Pet))
          selections:
            kg (alias for weight) {
              type guard: Exact(Dog)
            }
            kg (alias for weight) {
              type guard: Exact(Cat)
            }
            lb (alias for weight) {
              type guard: Exact(Dog)
            }
            g (alias for weight) {
              type guard: Exact(Dog)
            }
        }
        "###);
    }

    #[test]
    fn nested_redundant_fragments() {
        let plan = plan(
            SCHEMA_2,
            r#"
            query M_IdentityConditions {
              pet {
                ... on Pet {
                  ... on Pet {
                    name
                    ... on Animal { id }
                  }
                }
              }
            }"#,
        );

        insta::assert_snapshot!(plan, @"
        pet: {
          conditions: FieldType(OneOf(Cat, Dog, Pet))
          selections:
            name: {
            }
            id: {
              type guard: Exact(Cat)
            }
            id: {
              type guard: Exact(Dog)
            }
        }
        ");
    }

    /// Directive on a named fragment spread, with a field merged across two fragments
    #[test]
    fn directive_on_named_fragment_spread_with_field_merged_across_two_fragments() {
        let plan = plan(
            SCHEMA_2,
            r#"
            query SpreadDirective($withName: Boolean!, $deep: Boolean!) {
              node {
                id
                ...AnimalBits @include(if: $withName)
                ... on Dog @skip(if: $deep) { nickname }
              }
            }

            fragment AnimalBits on Animal {
              name
              ... on Dog { nickname }
            }"#,
        );

        insta::assert_snapshot!(plan, @"
        node: {
          conditions: FieldType(OneOf(Animal, Cat, Dog, Node, Owner, Pet, Robot))
          selections:
            id: {
            }
            name: {
              type guard: Exact(Cat)
              conditions: Include(if: $withName)
            }
            name: {
              type guard: Exact(Dog)
              conditions: Include(if: $withName)
            }
            nickname: {
              type guard: Exact(Dog)
              conditions: (Include(if: $withName) OR Skip(if: $deep))
            }
        }
        ");
    }
}
