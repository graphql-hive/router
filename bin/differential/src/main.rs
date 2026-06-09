use graphql_tools::ast::TypeDefinitionFields;
use graphql_tools::parser::schema::parse_schema;
use graphql_tools::parser::Pos;
use graphql_tools::parser::Style;
use graphql_tools::static_graphql::query::{
    self as q, Directive as QueryDirective, Document as QueryDocument, Field as QueryField,
    FragmentDefinition, FragmentSpread, InlineFragment, OperationDefinition, Selection,
    SelectionSet, Value as QueryValue, VariableDefinition,
};
use graphql_tools::static_graphql::schema::{
    self as s, Document as SchemaDocument, Type as SchemaType,
};
use rand::{prelude::IndexedRandom, rngs::StdRng, seq::SliceRandom, RngExt, SeedableRng};
use reqwest::Client;
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct GeneratorConfig {
    pub max_depth: usize,
    pub max_width: usize,
    pub max_fragments: usize,
    pub max_fragment_spreads: usize,
    pub max_inline_fragments: usize,
    pub max_directives: usize,
    pub alias_probability: f64,
    pub duplicate_field_probability: f64,
    pub named_fragment_probability: f64,
    pub inline_fragment_probability: f64,
    pub directive_probability: f64,
    pub variable_directive_probability: f64,
}

impl Default for GeneratorConfig {
    fn default() -> Self {
        Self {
            max_depth: 7,
            max_width: 6,
            max_fragments: 12,
            max_fragment_spreads: 24,
            max_inline_fragments: 24,
            max_directives: 48,
            alias_probability: 0.25,
            duplicate_field_probability: 0.18,
            named_fragment_probability: 0.35,
            inline_fragment_probability: 0.45,
            directive_probability: 0.45,
            variable_directive_probability: 0.65,
        }
    }
}

#[derive(Debug, Clone)]
pub struct QueryCase {
    pub document: String,
    pub operation_name: String,
    pub variables_json: String,
    pub features: FeatureCoverage,
}

#[derive(Debug, Clone, Default)]
pub struct FeatureCoverage {
    pub aliases: usize,
    pub duplicated_response_keys: usize,
    pub named_fragments: usize,
    pub fragment_spreads: usize,
    pub inline_fragments: usize,
    pub inline_fragments_without_type_condition: usize,
    pub skip_directives: usize,
    pub include_directives: usize,
    pub selections_with_both_skip_and_include: usize,
    pub directive_variables: usize,
    pub abstract_type_conditions: usize,
    pub concrete_type_conditions: usize,
    pub max_depth: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GenerationScope {
    All,
    Random,
    EquivalentFamilies,
}

impl GenerationScope {
    fn from_env() -> Self {
        match std::env::var("GRAPHQL_DIFF_MODE") {
            Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
                "all" | "mixed" => Self::All,
                "equivalent" | "family" | "families" | "equivalent-families" => {
                    Self::EquivalentFamilies
                }
                _ => Self::Random,
            },
            Err(_) => Self::All,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Random => "random",
            Self::EquivalentFamilies => "equivalent-families",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaseKind {
    Random,
    EquivalentFamily(EquivalentFamilyKind),
}

impl CaseKind {
    fn label(self) -> &'static str {
        match self {
            Self::Random => "random",
            Self::EquivalentFamily(kind) => kind.family_name(),
        }
    }
}

pub struct QueryGenerator<'a> {
    schema: &'a SchemaDocument,
    rng: StdRng,
    config: GeneratorConfig,
    fragments: Vec<FragmentDefinition>,
    variable_defs: BTreeMap<String, VariableDef>,
    variables: BTreeMap<String, bool>,
    counters: Counters,
    features: FeatureCoverage,
}

#[derive(Default)]
struct Counters {
    alias: usize,
    fragment: usize,
    variable: usize,
    fragment_spreads: usize,
    inline_fragments: usize,
    directives: usize,
}

#[derive(Debug, Clone)]
struct VariableDef {
    name: String,
    default_value: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionContext {
    Root,
    Field,
    FragmentDefinition,
    InlineFragment,
}

#[derive(Debug, Clone)]
struct QueryVariant {
    name: String,
    document: String,
}

#[derive(Debug, Clone)]
struct EquivalentQueryFamily {
    family_name: String,
    seed: u64,
    variants: Vec<QueryVariant>,
    variables_json: String,
    features: FeatureCoverage,
}

#[derive(Debug, Clone)]
enum RootFieldIntent {
    Rendered(String),
}

impl RootFieldIntent {
    fn render(&self) -> String {
        match self {
            Self::Rendered(field) => field.clone(),
        }
    }
}

#[derive(Debug, Clone)]
struct SemanticDirectiveIntent {
    include_variable: Option<String>,
    skip_variable: Option<String>,
}

#[derive(Debug, Clone)]
struct SemanticSelectionIntent {
    field_name: String,
}

#[derive(Debug, Clone)]
struct EquivalentSemanticIntent {
    operation_name: String,
    root_field: RootFieldIntent,
    target_field: String,
    abstract_type: String,
    concrete_type: String,
    shared_fields: Vec<SemanticSelectionIntent>,
    concrete_fields: Vec<SemanticSelectionIntent>,
    directives: SemanticDirectiveIntent,
}

#[derive(Debug, Clone)]
struct EquivalentFamilyCandidate {
    root_field: RootFieldIntent,
    target_field: String,
    abstract_type: String,
    concrete_type: String,
    shared_fields: Vec<String>,
    concrete_fields: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EquivalentFamilyKind {
    AbstractNamedVsConcreteNamed,
    AbstractNamedVsInlineOnly,
    NestedAbstractWithUntypedWrapper,
    DirectiveBearingEquivalent,
}

impl EquivalentFamilyKind {
    fn all() -> &'static [Self] {
        &[
            Self::AbstractNamedVsConcreteNamed,
            Self::AbstractNamedVsInlineOnly,
            Self::NestedAbstractWithUntypedWrapper,
            Self::DirectiveBearingEquivalent,
        ]
    }

    fn family_name(self) -> &'static str {
        match self {
            Self::AbstractNamedVsConcreteNamed => "abstract-named-vs-concrete-named",
            Self::AbstractNamedVsInlineOnly => "abstract-named-vs-inline-only",
            Self::NestedAbstractWithUntypedWrapper => "nested-abstract-with-untyped-wrapper",
            Self::DirectiveBearingEquivalent => "directive-bearing-equivalent",
        }
    }
}

struct EquivalentQueryFamilyGenerator<'a> {
    schema: &'a SchemaDocument,
    seed: u64,
}

#[derive(Debug, Clone)]
struct ExecutionResult {
    data: Option<JsonValue>,
    has_errors: bool,
    raw: JsonValue,
}

impl ExecutionResult {
    fn from_graphql_response(response: JsonValue) -> Self {
        match &response {
            JsonValue::Object(map) => Self {
                data: map.get("data").cloned(),
                has_errors: map.get("errors").is_some(),
                raw: response,
            },
            _ => panic!("Not a graphql response"),
        }
    }

    fn matches(&self, other: &Self) -> bool {
        self.data == other.data && self.has_errors == other.has_errors
    }
}

impl<'a> QueryGenerator<'a> {
    pub fn new(schema: &'a SchemaDocument, seed: u64, config: GeneratorConfig) -> Self {
        Self {
            schema,
            rng: StdRng::seed_from_u64(seed),
            config,
            fragments: Vec::new(),
            variable_defs: BTreeMap::new(),
            variables: BTreeMap::new(),
            counters: Counters::default(),
            features: FeatureCoverage::default(),
        }
    }

    pub fn generate(mut self) -> QueryCase {
        let operation_name = "GeneratedQuery".to_string();
        let root = self.schema.query_type_name().to_string();
        let selections = self.selection_set_for_type(&root, 0, SelectionContext::Root);

        let variables_json = self.render_variables_json();

        let mut defs = Vec::new();

        let mut query_vars = Vec::new();
        for def in self.variable_defs.values() {
            query_vars.push(VariableDefinition {
                position: Pos::default(),
                name: def.name.clone(),
                var_type: q::Type::NonNullType(Box::new(q::Type::NamedType("Boolean".to_string()))),
                default_value: def.default_value.map(QueryValue::Boolean),
            });
        }

        defs.push(q::Definition::Operation(OperationDefinition::Query(
            q::Query {
                position: Pos::default(),
                name: Some(operation_name.clone()),
                variable_definitions: query_vars,
                directives: Vec::new(),
                selection_set: SelectionSet {
                    span: (Pos::default(), Pos::default()),
                    items: selections,
                },
            },
        )));

        for frag in self.fragments {
            defs.push(q::Definition::Fragment(frag));
        }

        let doc = QueryDocument { definitions: defs };
        let style = Style::default();
        let document_str = doc.format(&style);

        QueryCase {
            document: document_str,
            operation_name,
            variables_json,
            features: self.features,
        }
    }

    fn selection_set_for_type(
        &mut self,
        type_name: &str,
        depth: usize,
        context: SelectionContext,
    ) -> Vec<Selection> {
        self.features.max_depth = self.features.max_depth.max(depth);

        if depth >= self.config.max_depth {
            return self.leafish_selection_set(type_name);
        }

        let type_def_opt = self.schema.type_by_name(type_name);
        let mut selections = Vec::new();

        if let Some(type_def) = type_def_opt {
            if type_def.is_composite_type()
                && (self.rng.random_bool(0.55) || type_def.is_union_type())
            {
                selections.push(Selection::Field(QueryField {
                    position: Pos::default(),
                    alias: None,
                    name: "__typename".to_string(),
                    arguments: Vec::new(),
                    directives: Vec::new(),
                    selection_set: SelectionSet {
                        span: (Pos::default(), Pos::default()),
                        items: Vec::new(),
                    },
                }));
            }

            if !type_def.is_union_type() {
                if let Some(TypeDefinitionFields::Fields(fields_slice)) = type_def.fields() {
                    let mut fields = fields_slice.to_vec();
                    fields.shuffle(&mut self.rng);

                    let width = self.rng.random_range(1..=self.config.max_width.max(1));
                    for field in fields.into_iter().take(width) {
                        selections.push(self.field_selection(&field, depth));

                        if self
                            .rng
                            .random_bool(self.config.duplicate_field_probability)
                        {
                            selections.push(self.field_selection(&field, depth));
                            self.features.duplicated_response_keys += 1;
                        }
                    }
                }
            }
        }

        if self.should_make_inline_fragment(context) {
            if let Some(inline_fragment) = self.inline_fragment(type_name, depth) {
                selections.push(Selection::InlineFragment(inline_fragment));
            }
        }

        if self.should_make_fragment_spread(context) {
            if let Some(fragment_spread) = self.fragment_spread(type_name, depth) {
                selections.push(Selection::FragmentSpread(fragment_spread));
            }
        }

        if selections.is_empty() {
            selections.push(Selection::Field(QueryField {
                position: Pos::default(),
                alias: None,
                name: "__typename".to_string(),
                arguments: Vec::new(),
                directives: Vec::new(),
                selection_set: SelectionSet {
                    span: (Pos::default(), Pos::default()),
                    items: Vec::new(),
                },
            }));
        }

        selections.shuffle(&mut self.rng);
        selections
    }

    fn leafish_selection_set(&mut self, type_name: &str) -> Vec<Selection> {
        let type_def_opt = self.schema.type_by_name(type_name);

        if let Some(type_def) = type_def_opt {
            if type_def.is_union_type() {
                return vec![Selection::Field(QueryField {
                    position: Pos::default(),
                    alias: None,
                    name: "__typename".to_string(),
                    arguments: Vec::new(),
                    directives: Vec::new(),
                    selection_set: SelectionSet {
                        span: (Pos::default(), Pos::default()),
                        items: Vec::new(),
                    },
                })];
            }
        }

        let mut selections = Vec::new();

        if self.rng.random_bool(0.60) {
            selections.push(Selection::Field(QueryField {
                position: Pos::default(),
                alias: None,
                name: "__typename".to_string(),
                arguments: Vec::new(),
                directives: Vec::new(),
                selection_set: SelectionSet {
                    span: (Pos::default(), Pos::default()),
                    items: Vec::new(),
                },
            }));
        }

        if let Some(type_def) = type_def_opt {
            if let Some(TypeDefinitionFields::Fields(fields_slice)) = type_def.fields() {
                let mut scalar_fields = fields_slice
                    .iter()
                    .filter(|field| self.is_leaf_output_type(&field.field_type))
                    .collect::<Vec<_>>();
                scalar_fields.shuffle(&mut self.rng);

                for field in scalar_fields
                    .into_iter()
                    .take(self.config.max_width.clamp(1, 3))
                {
                    selections.push(self.field_selection(field, self.config.max_depth));
                }
            }
        }

        if selections.is_empty() {
            selections.push(Selection::Field(QueryField {
                position: Pos::default(),
                alias: None,
                name: "__typename".to_string(),
                arguments: Vec::new(),
                directives: Vec::new(),
                selection_set: SelectionSet {
                    span: (Pos::default(), Pos::default()),
                    items: Vec::new(),
                },
            }));
        }

        selections
    }

    fn field_selection(&mut self, field: &s::Field, depth: usize) -> Selection {
        let named = field.field_type.inner_type().to_string();
        let is_composite = self
            .schema
            .type_by_name(&named)
            .map(|t| t.is_composite_type())
            .unwrap_or(false);

        let alias = if self.rng.random_bool(self.config.alias_probability) {
            self.features.aliases += 1;
            self.counters.alias += 1;
            Some(format!("a{}_{}", self.counters.alias, field.name))
        } else {
            None
        };

        let args = self.args_for_field(field);
        let directives = self.maybe_directives();
        let selection_set = if is_composite {
            self.selection_set_for_type(&named, depth + 1, SelectionContext::Field)
        } else {
            Vec::new()
        };

        Selection::Field(QueryField {
            position: Pos::default(),
            alias,
            name: field.name.clone(),
            arguments: args,
            directives,
            selection_set: SelectionSet {
                span: (Pos::default(), Pos::default()),
                items: selection_set,
            },
        })
    }

    fn args_for_field(&mut self, field: &s::Field) -> Vec<(String, QueryValue)> {
        let mut args = Vec::new();

        for arg in &field.arguments {
            let required = arg.value_type.is_non_null() && arg.default_value.is_none();

            if required || self.rng.random_bool(0.35) {
                if let Some(value) = self.literal_for_input_type(&arg.value_type) {
                    args.push((arg.name.clone(), value));
                }
            }
        }

        args
    }

    fn literal_for_input_type(&mut self, ty: &SchemaType) -> Option<QueryValue> {
        match ty {
            SchemaType::NonNullType(inner) => self.literal_for_input_type(inner),
            SchemaType::ListType(inner) => {
                let len = self.rng.random_range(0..=3);
                let mut values = Vec::new();
                for _ in 0..len {
                    if let Some(v) = self.literal_for_input_type(inner) {
                        values.push(v);
                    }
                }
                Some(QueryValue::List(values))
            }
            SchemaType::NamedType(name) => match name.as_str() {
                "ID" => Some(QueryValue::String(format!(
                    "id-{}",
                    self.rng.random_range(0..1000)
                ))),
                "String" => Some(QueryValue::String(format!(
                    "s{}",
                    self.rng.random_range(0..1000)
                ))),
                "Int" => Some(QueryValue::Int(self.rng.random_range(0..100).into())),
                "Float" => Some(QueryValue::Float(self.rng.random_range(0.0..100.0))),
                "Boolean" => Some(QueryValue::Boolean(self.rng.random_bool(0.5))),
                other => {
                    let type_def = self.schema.type_by_name(other);
                    if let Some(type_def) = type_def {
                        if type_def.is_enum_type() {
                            if let Some(TypeDefinitionFields::EnumValues(values)) =
                                type_def.fields()
                            {
                                if let Some(v) = values.choose(&mut self.rng) {
                                    return Some(QueryValue::Enum(v.name.clone()));
                                }
                            }
                        }
                    }
                    None
                }
            },
        }
    }

    fn should_make_fragment_spread(&self, context: SelectionContext) -> bool {
        !matches!(context, SelectionContext::FragmentDefinition)
            && self.counters.fragment_spreads < self.config.max_fragment_spreads
            && self.fragments.len() < self.config.max_fragments
            && self.rng_bool_peekable(self.config.named_fragment_probability)
    }

    fn should_make_inline_fragment(&self, _context: SelectionContext) -> bool {
        self.counters.inline_fragments < self.config.max_inline_fragments
            && self.rng_bool_peekable(self.config.inline_fragment_probability)
    }

    fn rng_bool_peekable(&self, probability: f64) -> bool {
        probability > 0.0
    }

    fn fragment_spread(&mut self, current_type: &str, depth: usize) -> Option<FragmentSpread> {
        if !self.rng.random_bool(self.config.named_fragment_probability) {
            return None;
        }

        self.counters.fragment_spreads += 1;
        self.features.fragment_spreads += 1;

        let type_condition = self.compatible_type_condition(current_type)?;
        self.counters.fragment += 1;
        let name = format!("GeneratedFragment{}", self.counters.fragment);

        let fragment_selection_set = self.selection_set_for_type(
            &type_condition,
            depth + 1,
            SelectionContext::FragmentDefinition,
        );

        self.record_type_condition_feature(&type_condition);
        self.fragments.push(FragmentDefinition {
            position: Pos::default(),
            name: name.clone(),
            type_condition: q::TypeCondition::On(type_condition),
            directives: Vec::new(),
            selection_set: SelectionSet {
                span: (Pos::default(), Pos::default()),
                items: fragment_selection_set,
            },
        });
        self.features.named_fragments += 1;

        Some(FragmentSpread {
            position: Pos::default(),
            fragment_name: name,
            directives: self.maybe_directives(),
        })
    }

    fn inline_fragment(&mut self, current_type: &str, depth: usize) -> Option<InlineFragment> {
        if !self
            .rng
            .random_bool(self.config.inline_fragment_probability)
        {
            return None;
        }

        self.counters.inline_fragments += 1;
        self.features.inline_fragments += 1;

        let no_type_condition = self.rng.random_bool(0.25);
        let type_condition = if no_type_condition {
            self.features.inline_fragments_without_type_condition += 1;
            None
        } else {
            let ty = self.compatible_type_condition(current_type)?;
            self.record_type_condition_feature(&ty);
            Some(ty)
        };

        let scoped_type = type_condition
            .clone()
            .unwrap_or_else(|| current_type.to_string());
        let selection_set =
            self.selection_set_for_type(&scoped_type, depth + 1, SelectionContext::InlineFragment);

        Some(InlineFragment {
            position: Pos::default(),
            type_condition: type_condition.map(q::TypeCondition::On),
            directives: self.maybe_directives(),
            selection_set: SelectionSet {
                span: (Pos::default(), Pos::default()),
                items: selection_set,
            },
        })
    }

    fn compatible_type_condition(&mut self, current_type: &str) -> Option<String> {
        let current_def = self.schema.type_by_name(current_type)?;

        if current_def.is_object_type() {
            Some(current_type.to_string())
        } else if current_def.is_abstract_type() {
            let mut candidates: Vec<String> = current_def
                .possible_types(self.schema)
                .iter()
                .map(|t| t.name().to_string())
                .collect();

            if matches!(current_def, s::TypeDefinition::Interface(_)) {
                candidates.push(current_type.to_string());
            }

            candidates.sort();
            candidates.dedup();
            candidates.choose(&mut self.rng).cloned()
        } else {
            None
        }
    }

    fn record_type_condition_feature(&mut self, type_condition: &str) {
        if let Some(def) = self.schema.type_by_name(type_condition) {
            if def.is_abstract_type() {
                self.features.abstract_type_conditions += 1;
            } else if def.is_object_type() {
                self.features.concrete_type_conditions += 1;
            }
        }
    }

    fn maybe_directives(&mut self) -> Vec<QueryDirective> {
        if self.counters.directives >= self.config.max_directives {
            return Vec::new();
        }

        if !self.rng.random_bool(self.config.directive_probability) {
            return Vec::new();
        }

        let mode = self.rng.random_range(0..=4);
        let mut directives = Vec::new();

        match mode {
            0 => directives.push(self.directive("skip")),
            1 => directives.push(self.directive("include")),
            2 | 3 => {
                directives.push(self.directive("skip"));
                directives.push(self.directive("include"));
                self.features.selections_with_both_skip_and_include += 1;
            }
            _ => {
                directives.push(self.directive("include"));
                directives.push(self.directive("skip"));
                self.features.selections_with_both_skip_and_include += 1;
            }
        }

        directives
    }

    fn directive(&mut self, name: &'static str) -> QueryDirective {
        self.counters.directives += 1;

        match name {
            "skip" => self.features.skip_directives += 1,
            "include" => self.features.include_directives += 1,
            _ => {}
        }

        let value = match name {
            "skip" => self.rng.random_bool(0.20),
            "include" => self.rng.random_bool(0.80),
            _ => self.rng.random_bool(0.50),
        };

        let arg = if self
            .rng
            .random_bool(self.config.variable_directive_probability)
        {
            self.features.directive_variables += 1;
            QueryValue::Variable(self.bool_variable(value))
        } else {
            QueryValue::Boolean(value)
        };

        QueryDirective {
            position: Pos::default(),
            name: name.to_string(),
            arguments: vec![("if".to_string(), arg)],
        }
    }

    fn bool_variable(&mut self, value: bool) -> String {
        self.counters.variable += 1;
        let name = format!("v{}", self.counters.variable);

        let with_default = self.rng.random_bool(0.35);
        let omit_from_variables = with_default && self.rng.random_bool(0.45);

        self.variable_defs.insert(
            name.clone(),
            VariableDef {
                name: name.clone(),
                default_value: with_default.then_some(value),
            },
        );

        if !omit_from_variables {
            self.variables.insert(name.clone(), value);
        }

        name
    }

    fn is_leaf_output_type(&self, ty: &SchemaType) -> bool {
        let named = ty.inner_type();
        if let Some(def) = self.schema.type_by_name(named) {
            return def.is_scalar_type() || def.is_enum_type();
        }
        false
    }

    fn render_variables_json(&self) -> String {
        render_bool_variables_json(&self.variables)
    }
}

impl<'a> EquivalentQueryFamilyGenerator<'a> {
    fn new(schema: &'a SchemaDocument, seed: u64) -> Self {
        Self { schema, seed }
    }

    fn generate(&self, family_kind: EquivalentFamilyKind) -> Option<EquivalentQueryFamily> {
        let intent = self.semantic_intent_for(family_kind)?;

        Some(match family_kind {
            EquivalentFamilyKind::AbstractNamedVsConcreteNamed => {
                self.render_abstract_named_vs_concrete_named(family_kind, &intent)
            }
            EquivalentFamilyKind::AbstractNamedVsInlineOnly => {
                self.render_abstract_named_vs_inline_only(family_kind, &intent)
            }
            EquivalentFamilyKind::NestedAbstractWithUntypedWrapper => {
                self.render_nested_abstract_with_untyped_wrapper(family_kind, &intent)
            }
            EquivalentFamilyKind::DirectiveBearingEquivalent => {
                self.render_directive_bearing_equivalent(family_kind, &intent)
            }
        })
    }

    fn semantic_intent_for(
        &self,
        family_kind: EquivalentFamilyKind,
    ) -> Option<EquivalentSemanticIntent> {
        let candidate = self.equivalent_family_candidate()?;

        let directives = match family_kind {
            EquivalentFamilyKind::DirectiveBearingEquivalent => SemanticDirectiveIntent {
                include_variable: Some("includeShared".to_string()),
                skip_variable: Some("skipNever".to_string()),
            },
            _ => SemanticDirectiveIntent {
                include_variable: None,
                skip_variable: None,
            },
        };

        Some(EquivalentSemanticIntent {
            operation_name: format!("EquivalentFamily{}", self.seed),
            root_field: candidate.root_field,
            target_field: candidate.target_field,
            abstract_type: candidate.abstract_type,
            concrete_type: candidate.concrete_type,
            shared_fields: candidate
                .shared_fields
                .into_iter()
                .map(|field_name| SemanticSelectionIntent { field_name })
                .collect(),
            concrete_fields: candidate
                .concrete_fields
                .into_iter()
                .map(|field_name| SemanticSelectionIntent { field_name })
                .collect(),
            directives,
        })
    }

    fn equivalent_family_candidate(&self) -> Option<EquivalentFamilyCandidate> {
        let query_type_name = self.schema.query_type_name();
        let query_type = self.schema.type_by_name(query_type_name)?;
        let TypeDefinitionFields::Fields(query_fields) = query_type.fields()? else {
            return None;
        };

        let mut candidates = Vec::new();

        for query_field in query_fields {
            let root_field_type_name = query_field.field_type.inner_type();
            let Some(root_type) = self.schema.type_by_name(root_field_type_name) else {
                continue;
            };
            let Some(root_field_intent) = render_root_field(query_field) else {
                continue;
            };

            let TypeDefinitionFields::Fields(root_fields) = root_type.fields()? else {
                continue;
            };

            for root_field in root_fields {
                let abstract_type_name = root_field.field_type.inner_type();
                let Some(abstract_type) = self.schema.type_by_name(abstract_type_name) else {
                    continue;
                };

                if !abstract_type.is_abstract_type() {
                    continue;
                }

                let shared_fields = collect_leaf_field_names(abstract_type);
                if shared_fields.is_empty() {
                    continue;
                }

                for concrete_type in abstract_type.possible_types(self.schema) {
                    let concrete_fields =
                        collect_extra_leaf_field_names(concrete_type, &shared_fields);
                    if concrete_fields.is_empty() {
                        continue;
                    }

                    candidates.push(EquivalentFamilyCandidate {
                        root_field: root_field_intent.clone(),
                        target_field: root_field.name.clone(),
                        abstract_type: abstract_type.name().to_string(),
                        concrete_type: concrete_type.name().to_string(),
                        shared_fields: shared_fields.clone(),
                        concrete_fields,
                    });
                }
            }
        }

        if candidates.is_empty() {
            return None;
        }

        let index = (self.seed as usize) % candidates.len();
        candidates.into_iter().nth(index)
    }

    fn render_abstract_named_vs_concrete_named(
        &self,
        family_kind: EquivalentFamilyKind,
        intent: &EquivalentSemanticIntent,
    ) -> EquivalentQueryFamily {
        let abstract_fragment = format!(
            "fragment Test on {} {{\n  ... on {} {{\n{}\n  }}\n  ... on {} {{\n{}\n  }}\n}}",
            intent.abstract_type,
            intent.abstract_type,
            indent_lines(&render_fields(&intent.shared_fields), 4),
            intent.concrete_type,
            indent_lines(&render_fields(&intent.concrete_fields), 4)
        );

        let concrete_fragment = format!(
            "fragment Test on {} {{\n  ... on {} {{\n{}\n  }}\n{}\n}}",
            intent.concrete_type,
            intent.abstract_type,
            indent_lines(&render_fields(&intent.shared_fields), 4),
            indent_lines(&render_fields(&intent.concrete_fields), 2)
        );

        EquivalentQueryFamily {
            family_name: family_kind.family_name().to_string(),
            seed: self.seed,
            variants: vec![
                QueryVariant {
                    name: "abstract-named".to_string(),
                    document: self.render_query(
                        intent,
                        None,
                        &[abstract_fragment],
                        "        ...Test\n",
                    ),
                },
                QueryVariant {
                    name: "concrete-named".to_string(),
                    document: self.render_query(
                        intent,
                        None,
                        &[concrete_fragment],
                        "        ...Test\n",
                    ),
                },
            ],
            variables_json: "{}".to_string(),
            features: FeatureCoverage {
                named_fragments: 2,
                fragment_spreads: 2,
                inline_fragments: 4,
                abstract_type_conditions: 2,
                concrete_type_conditions: 3,
                max_depth: 5,
                ..FeatureCoverage::default()
            },
        }
    }

    fn render_abstract_named_vs_inline_only(
        &self,
        family_kind: EquivalentFamilyKind,
        intent: &EquivalentSemanticIntent,
    ) -> EquivalentQueryFamily {
        let abstract_fragment = format!(
            "fragment Test on {} {{\n  ... on {} {{\n{}\n  }}\n  ... on {} {{\n{}\n  }}\n}}",
            intent.abstract_type,
            intent.abstract_type,
            indent_lines(&render_fields(&intent.shared_fields), 4),
            intent.concrete_type,
            indent_lines(&render_fields(&intent.concrete_fields), 4)
        );

        let inline_variant = format!(
            "        ... on {} {{\n          ... on {} {{\n{}\n          }}\n          ... on {} {{\n{}\n          }}\n        }}\n",
            intent.abstract_type,
            intent.abstract_type,
            indent_lines(&render_fields(&intent.shared_fields), 12),
            intent.concrete_type,
            indent_lines(&render_fields(&intent.concrete_fields), 12)
        );

        EquivalentQueryFamily {
            family_name: family_kind.family_name().to_string(),
            seed: self.seed,
            variants: vec![
                QueryVariant {
                    name: "abstract-named".to_string(),
                    document: self.render_query(
                        intent,
                        None,
                        &[abstract_fragment],
                        "        ...Test\n",
                    ),
                },
                QueryVariant {
                    name: "inline-only".to_string(),
                    document: self.render_query(intent, None, &[], &inline_variant),
                },
            ],
            variables_json: "{}".to_string(),
            features: FeatureCoverage {
                named_fragments: 1,
                fragment_spreads: 1,
                inline_fragments: 6,
                abstract_type_conditions: 3,
                concrete_type_conditions: 3,
                max_depth: 5,
                ..FeatureCoverage::default()
            },
        }
    }

    fn render_nested_abstract_with_untyped_wrapper(
        &self,
        family_kind: EquivalentFamilyKind,
        intent: &EquivalentSemanticIntent,
    ) -> EquivalentQueryFamily {
        let named_fragment = format!(
            "fragment Test on {} {{\n  ... on {} {{\n    ... {{\n{}\n    }}\n  }}\n  ... on {} {{\n{}\n  }}\n}}",
            intent.abstract_type,
            intent.abstract_type,
            indent_lines(&render_fields(&intent.shared_fields), 6),
            intent.concrete_type,
            indent_lines(&render_fields(&intent.concrete_fields), 4)
        );

        let inline_variant = format!(
            "        ... on {} {{\n          ... {{\n            ... on {} {{\n{}\n            }}\n          }}\n          ... on {} {{\n{}\n          }}\n        }}\n",
            intent.abstract_type,
            intent.abstract_type,
            indent_lines(&render_fields(&intent.shared_fields), 14),
            intent.concrete_type,
            indent_lines(&render_fields(&intent.concrete_fields), 12)
        );

        EquivalentQueryFamily {
            family_name: family_kind.family_name().to_string(),
            seed: self.seed,
            variants: vec![
                QueryVariant {
                    name: "named-nested".to_string(),
                    document: self.render_query(
                        intent,
                        None,
                        &[named_fragment],
                        "        ...Test\n",
                    ),
                },
                QueryVariant {
                    name: "inline-untyped-wrapper".to_string(),
                    document: self.render_query(intent, None, &[], &inline_variant),
                },
            ],
            variables_json: "{}".to_string(),
            features: FeatureCoverage {
                named_fragments: 1,
                fragment_spreads: 1,
                inline_fragments: 7,
                inline_fragments_without_type_condition: 2,
                abstract_type_conditions: 3,
                concrete_type_conditions: 2,
                max_depth: 6,
                ..FeatureCoverage::default()
            },
        }
    }

    fn render_directive_bearing_equivalent(
        &self,
        family_kind: EquivalentFamilyKind,
        intent: &EquivalentSemanticIntent,
    ) -> EquivalentQueryFamily {
        let include_var = intent
            .directives
            .include_variable
            .as_deref()
            .expect("directive family requires include variable");
        let skip_var = intent
            .directives
            .skip_variable
            .as_deref()
            .expect("directive family requires skip variable");

        let named_fragment = format!(
            "fragment Test on {} {{\n  ... on {} @skip(if: ${}) {{\n{}\n  }}\n  ... on {} @include(if: ${}) {{\n{}\n  }}\n}}",
            intent.abstract_type,
            intent.abstract_type,
            skip_var,
            indent_lines(&render_fields(&intent.shared_fields), 4),
            intent.concrete_type,
            include_var,
            indent_lines(&render_fields(&intent.concrete_fields), 4)
        );

        let inline_variant = format!(
            "        ... on {} @include(if: ${}) {{\n          ... @skip(if: ${}) {{\n{}\n          }}\n        }}\n        ... on {} @include(if: ${}) {{\n{}\n        }}\n",
            intent.abstract_type,
            include_var,
            skip_var,
            indent_lines(&render_fields(&intent.shared_fields), 12),
            intent.concrete_type,
            include_var,
            indent_lines(&render_fields(&intent.concrete_fields), 10)
        );

        let mut variables = BTreeMap::new();
        variables.insert(include_var.to_string(), true);
        variables.insert(skip_var.to_string(), false);

        EquivalentQueryFamily {
            family_name: family_kind.family_name().to_string(),
            seed: self.seed,
            variants: vec![
                QueryVariant {
                    name: "directive-named".to_string(),
                    document: self.render_query(
                        intent,
                        Some("$includeShared: Boolean!, $skipNever: Boolean!"),
                        &[named_fragment],
                        "        ...Test @include(if: $includeShared)\n",
                    ),
                },
                QueryVariant {
                    name: "directive-inline".to_string(),
                    document: self.render_query(
                        intent,
                        Some("$includeShared: Boolean!, $skipNever: Boolean!"),
                        &[],
                        &inline_variant,
                    ),
                },
            ],
            variables_json: render_bool_variables_json(&variables),
            features: FeatureCoverage {
                named_fragments: 1,
                fragment_spreads: 1,
                inline_fragments: 6,
                skip_directives: 2,
                include_directives: 4,
                selections_with_both_skip_and_include: 1,
                directive_variables: 2,
                abstract_type_conditions: 2,
                concrete_type_conditions: 2,
                max_depth: 5,
                ..FeatureCoverage::default()
            },
        }
    }

    fn render_query(
        &self,
        intent: &EquivalentSemanticIntent,
        variable_defs: Option<&str>,
        fragments: &[String],
        twitter_branch_body: &str,
    ) -> String {
        let operation_name = &intent.operation_name;
        let operation_header = match variable_defs {
            Some(variable_defs) => format!("query {}({})", operation_name, variable_defs),
            None => format!("query {}", operation_name),
        };
        let root_field = intent.root_field.render();

        let mut document = format!(
            "{} {{\n  {} {{\n    {} {{\n      __typename\n      ... on {} {{\n{}      }}\n    }}\n  }}\n}}",
            operation_header,
            root_field,
            intent.target_field,
            intent.concrete_type,
            twitter_branch_body
        );

        for fragment in fragments {
            document.push_str("\n\n");
            document.push_str(fragment);
        }

        document
    }
}

fn render_fields(fields: &[SemanticSelectionIntent]) -> String {
    fields
        .iter()
        .map(|field| field.field_name.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn indent_lines(text: &str, spaces: usize) -> String {
    let indent = " ".repeat(spaces);
    text.lines()
        .map(|line| format!("{}{}", indent, line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_bool_variables_json(variables: &BTreeMap<String, bool>) -> String {
    let body = variables
        .iter()
        .map(|(key, value)| format!("\"{}\": {}", key, value))
        .collect::<Vec<_>>()
        .join(", ");

    format!("{{{}}}", body)
}

fn collect_leaf_field_names(type_def: &s::TypeDefinition) -> Vec<String> {
    match type_def.fields() {
        Some(TypeDefinitionFields::Fields(fields)) => fields
            .iter()
            .filter(|field| {
                matches!(
                    field.field_type.inner_type(),
                    "String" | "Int" | "Float" | "Boolean" | "ID"
                )
            })
            .map(|field| field.name.clone())
            .collect(),
        _ => Vec::new(),
    }
}

fn collect_extra_leaf_field_names(
    concrete_type: &s::TypeDefinition,
    shared_fields: &[String],
) -> Vec<String> {
    match concrete_type.fields() {
        Some(TypeDefinitionFields::Fields(fields)) => fields
            .iter()
            .filter(|field| {
                matches!(
                    field.field_type.inner_type(),
                    "String" | "Int" | "Float" | "Boolean" | "ID"
                ) && !shared_fields.iter().any(|shared| shared == &field.name)
            })
            .map(|field| field.name.clone())
            .collect(),
        _ => Vec::new(),
    }
}

fn render_root_field(field: &s::Field) -> Option<RootFieldIntent> {
    let mut rendered = field.name.clone();

    if !field.arguments.is_empty() {
        let mut rendered_args = Vec::new();
        for arg in &field.arguments {
            let required = arg.value_type.is_non_null() && arg.default_value.is_none();
            if !required {
                continue;
            }

            let value = match arg.value_type.inner_type() {
                "ID" => format!("\"id-1\""),
                "String" => format!("\"s1\""),
                "Int" => "1".to_string(),
                "Float" => "1.0".to_string(),
                "Boolean" => "true".to_string(),
                _ => return None,
            };
            rendered_args.push(format!("{}: {}", arg.name, value));
        }

        if !rendered_args.is_empty() {
            rendered.push('(');
            rendered.push_str(&rendered_args.join(", "));
            rendered.push(')');
        }
    }

    Some(RootFieldIntent::Rendered(rendered))
}

fn scheduled_case_kinds(scope: GenerationScope) -> Vec<CaseKind> {
    match scope {
        GenerationScope::All => {
            let mut kinds = Vec::with_capacity(1 + EquivalentFamilyKind::all().len());
            kinds.push(CaseKind::Random);
            kinds.extend(
                EquivalentFamilyKind::all()
                    .iter()
                    .copied()
                    .map(CaseKind::EquivalentFamily),
            );
            kinds
        }
        GenerationScope::Random => vec![CaseKind::Random],
        GenerationScope::EquivalentFamilies => EquivalentFamilyKind::all()
            .iter()
            .copied()
            .map(CaseKind::EquivalentFamily)
            .collect(),
    }
}

async fn execute_query(
    client: &Client,
    url: &str,
    query: &str,
    variables: &str,
) -> Result<JsonValue, reqwest::Error> {
    let body = serde_json::json!({
        "query": query,
        "variables": serde_json::from_str::<JsonValue>(variables).unwrap_or(serde_json::json!({}))
    });

    let res = client
        .post(url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;

    res.json::<JsonValue>().await
}

fn persist_random_failure(
    case_index: usize,
    case: &QueryCase,
    baseline: &JsonValue,
    candidate: &JsonValue,
) {
    let dir = format!("./failed-tests/case-{}", case_index);
    std::fs::create_dir_all(&dir).expect("to create a directory");
    std::fs::write(format!("{}/query.graphql", dir), case.document.clone())
        .expect("to create query.graphql");
    std::fs::write(
        format!("{}/variables.json", dir),
        case.variables_json.clone(),
    )
    .expect("to create variables.json");
    std::fs::write(
        format!("{}/endpoint-1.json", dir),
        serde_json::to_string_pretty(baseline).unwrap(),
    )
    .expect("to create endpoint-1.json");
    std::fs::write(
        format!("{}/endpoint-2.json", dir),
        serde_json::to_string_pretty(candidate).unwrap(),
    )
    .expect("to create endpoint-2.json");
}

fn persist_equivalent_failure(
    case_index: usize,
    family: &EquivalentQueryFamily,
    baseline_results: &[ExecutionResult],
    candidate_results: &[ExecutionResult],
    mismatch_reason: &str,
) {
    let dir = format!("./failed-tests/case-{}", case_index);
    std::fs::create_dir_all(&dir).expect("to create a directory");
    std::fs::write(format!("{}/family.txt", dir), family.family_name.as_str())
        .expect("to create family.txt");
    std::fs::write(format!("{}/seed.txt", dir), family.seed.to_string())
        .expect("to create seed.txt");
    std::fs::write(format!("{}/mismatch.txt", dir), mismatch_reason)
        .expect("to create mismatch.txt");
    std::fs::write(
        format!("{}/variables.json", dir),
        family.variables_json.as_str(),
    )
    .expect("to create variables.json");

    for (index, variant) in family.variants.iter().enumerate() {
        let label = (b'a' + index as u8) as char;
        std::fs::write(
            format!("{}/variant-{}.graphql", dir, label),
            variant.document.as_str(),
        )
        .expect("to create variant query");
        std::fs::write(
            format!("{}/baseline-{}.json", dir, label),
            serde_json::to_string_pretty(&baseline_results[index].raw).unwrap(),
        )
        .expect("to create baseline response");
        std::fs::write(
            format!("{}/candidate-{}.json", dir, label),
            serde_json::to_string_pretty(&candidate_results[index].raw).unwrap(),
        )
        .expect("to create candidate response");
    }
}

async fn run_random_case(
    client: &Client,
    baseline_endpoint: &str,
    candidate_endpoint: &str,
    case_index: usize,
    seed: u64,
    schema: &SchemaDocument,
) -> bool {
    let case = QueryGenerator::new(schema, seed, GeneratorConfig::default()).generate();

    println!("Query #{}:", case_index + 1);
    println!("  kind: {}", CaseKind::Random.label());
    println!("  operation: {}", case.operation_name);
    println!(
        "  features: named_fragments={}, inline_fragments={}, abstract_conditions={}, concrete_conditions={}",
        case.features.named_fragments,
        case.features.inline_fragments,
        case.features.abstract_type_conditions,
        case.features.concrete_type_conditions
    );

    let res1_future = execute_query(
        client,
        baseline_endpoint,
        &case.document,
        &case.variables_json,
    );
    let res2_future = execute_query(
        client,
        candidate_endpoint,
        &case.document,
        &case.variables_json,
    );

    let (res1, res2) = tokio::join!(res1_future, res2_future);

    let success = match (res1, res2) {
        (Ok(res1), Ok(res2)) => {
            let baseline = ExecutionResult::from_graphql_response(res1.clone());
            let candidate = ExecutionResult::from_graphql_response(res2.clone());

            if !baseline.matches(&candidate) {
                persist_random_failure(case_index, &case, &res1, &res2);
                println!("⚠️ Responses differ");
                false
            } else {
                println!("✅ Responses match");
                true
            }
        }
        (Err(e1), Err(e2)) => {
            println!("⚠️ Both endpoints failed");
            println!("  baseline: {}", e1);
            println!("  candidate: {}", e2);
            true
        }
        (Err(e1), Ok(_)) => {
            println!("❌ Baseline endpoint failed: {}", e1);
            false
        }
        (Ok(_), Err(e2)) => {
            println!("❌ Candidate endpoint failed: {}", e2);
            false
        }
    };

    println!("--------------------------------------------------");
    success
}

async fn run_equivalent_family_case(
    client: &Client,
    baseline_endpoint: &str,
    candidate_endpoint: &str,
    case_index: usize,
    seed: u64,
    family_kind: EquivalentFamilyKind,
    schema: &SchemaDocument,
) -> bool {
    let Some(family) = EquivalentQueryFamilyGenerator::new(schema, seed).generate(family_kind)
    else {
        println!("Family #{}:", case_index + 1);
        println!("❌ Schema does not support equivalent families");
        println!("--------------------------------------------------");
        return false;
    };

    println!("Family #{}:", case_index + 1);
    println!(
        "  kind: {}",
        CaseKind::EquivalentFamily(family_kind).label()
    );
    println!("  name: {}", family.family_name);
    println!("  variants: {}", family.variants.len());
    println!(
        "  features: named_fragments={}, inline_fragments={}, directives(include={}, skip={})",
        family.features.named_fragments,
        family.features.inline_fragments,
        family.features.include_directives,
        family.features.skip_directives,
    );

    let mut baseline_results = Vec::with_capacity(family.variants.len());
    let mut candidate_results = Vec::with_capacity(family.variants.len());

    for variant in &family.variants {
        let baseline_future = execute_query(
            client,
            baseline_endpoint,
            &variant.document,
            &family.variables_json,
        );
        let candidate_future = execute_query(
            client,
            candidate_endpoint,
            &variant.document,
            &family.variables_json,
        );

        let (baseline_res, candidate_res) = tokio::join!(baseline_future, candidate_future);

        let (baseline, candidate) = match (baseline_res, candidate_res) {
            (Ok(baseline_res), Ok(candidate_res)) => (
                ExecutionResult::from_graphql_response(baseline_res),
                ExecutionResult::from_graphql_response(candidate_res),
            ),
            (Err(e1), Err(e2)) => {
                println!("⚠️ Both endpoints failed for variant {}", variant.name);
                println!("  baseline: {}", e1);
                println!("  candidate: {}", e2);
                println!("--------------------------------------------------");
                return true;
            }
            (Err(e1), Ok(_)) => {
                println!(
                    "❌ Baseline endpoint failed for variant {}: {}",
                    variant.name, e1
                );
                println!("--------------------------------------------------");
                return false;
            }
            (Ok(_), Err(e2)) => {
                println!(
                    "❌ Candidate endpoint failed for variant {}: {}",
                    variant.name, e2
                );
                println!("--------------------------------------------------");
                return false;
            }
        };

        baseline_results.push(baseline);
        candidate_results.push(candidate);
    }

    for (index, (baseline, candidate)) in baseline_results
        .iter()
        .zip(candidate_results.iter())
        .enumerate()
    {
        if !baseline.matches(candidate) {
            let reason = format!(
                "baseline-vs-candidate mismatch for variant {} ({})",
                index, family.variants[index].name
            );
            persist_equivalent_failure(
                case_index,
                &family,
                &baseline_results,
                &candidate_results,
                &reason,
            );
            println!(
                "⚠️ Baseline and candidate differ for variant {}",
                family.variants[index].name
            );
            println!("--------------------------------------------------");
            return false;
        }
    }

    for left in 0..candidate_results.len() {
        for right in (left + 1)..candidate_results.len() {
            if !candidate_results[left].matches(&candidate_results[right]) {
                let reason = format!(
                    "candidate intra-family mismatch between variant {} ({}) and variant {} ({})",
                    left, family.variants[left].name, right, family.variants[right].name,
                );
                persist_equivalent_failure(
                    case_index,
                    &family,
                    &baseline_results,
                    &candidate_results,
                    &reason,
                );
                println!(
                    "⚠️ Candidate variants {} and {} are not equivalent",
                    family.variants[left].name, family.variants[right].name
                );
                println!("--------------------------------------------------");
                return false;
            }
        }
    }

    println!("✅ Family matches across endpoints and variants");
    println!("--------------------------------------------------");
    true
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!(
            "Usage: {} <baseline-endpoint> <candidate-endpoint> <schema.graphql>",
            args[0]
        );
        eprintln!(
            "Example: {} http://localhost:4300/graphql http://localhost:4000/graphql bench/schema.graphql",
            args[0]
        );
        eprintln!(
            "Mode is controlled with GRAPHQL_DIFF_MODE=all|random|equivalent-families (default: all)"
        );
        return;
    }

    let baseline_endpoint = &args[1];
    let candidate_endpoint = &args[2];

    let schema_str = match std::fs::read_to_string(&args[3]) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("Failed to read schema file {}: {}", args[3], e);
            return;
        }
    };

    let schema = match parse_schema::<String>(&schema_str) {
        Ok(doc) => doc.into_static(),
        Err(e) => {
            eprintln!("Failed to parse schema: {}", e);
            return;
        }
    };

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    let mut differences = 0;
    let mut num_queries = 100;

    if let Ok(val) = std::env::var("GRAPHQL_DIFF_QUERIES") {
        num_queries = val.parse().unwrap_or(10);
    }

    let scope = GenerationScope::from_env();
    let case_kinds = scheduled_case_kinds(scope);

    println!("Running {} differential cases against:", num_queries);
    println!("  mode: {}", scope.as_str());
    println!(
        "  scheduled kinds: {}",
        case_kinds
            .iter()
            .map(|kind| kind.label())
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("  baseline: {}", baseline_endpoint);
    println!("  candidate: {}", candidate_endpoint);
    println!("--------------------------------------------------");

    for i in 0..num_queries {
        let seed = i as u64 + 42;
        let case_kind = case_kinds[i % case_kinds.len()];
        let success = match case_kind {
            CaseKind::Random => {
                run_random_case(
                    &client,
                    baseline_endpoint,
                    candidate_endpoint,
                    i,
                    seed,
                    &schema,
                )
                .await
            }
            CaseKind::EquivalentFamily(family_kind) => {
                run_equivalent_family_case(
                    &client,
                    baseline_endpoint,
                    candidate_endpoint,
                    i,
                    seed,
                    family_kind,
                    &schema,
                )
                .await
            }
        };

        if !success {
            differences += 1;
        }
    }

    if differences == 0 {
        println!("🎉 All cases returned matching results!");
    } else {
        println!("⚠️ Found {} cases with different results.", differences);
        std::process::exit(1);
    }
}
