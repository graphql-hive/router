//! Makes random operations for a schema, see `QueryGenerator`.

use graphql_tools::ast::TypeDefinitionFields;
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
use std::collections::BTreeMap;

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
    /// How likely an inline fragment gets `@defer`. 0 by default, then the generator draws
    /// nothing extra, so a seed gives the same operation it always did.
    pub defer_probability: f64,
    /// At most this many `@defer`s per operation, the router's `limits.max_defers`.
    pub max_defers: usize,
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
            defer_probability: 0.0,
            max_defers: 4,
        }
    }
}

#[derive(Debug, Clone)]
pub struct QueryCase {
    pub document: String,
    pub operation_name: String,
    pub variables_json: String,
    /// `document` without its `@defer`s, and `variables_json` without their variables. The same
    /// as those when there's no `@defer`. Merging the parts of `document`'s response gives
    /// the response to this one.
    pub plain_document: String,
    pub plain_variables_json: String,
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
    pub defers: usize,
    pub max_depth: usize,
}

pub struct QueryGenerator<'a> {
    schema: &'a SchemaDocument,
    rng: StdRng,
    config: GeneratorConfig,
    fragments: Vec<FragmentDefinition>,
    variable_defs: BTreeMap<String, VariableDef>,
    variables: BTreeMap<String, bool>,
    /// The `@defer(if:)` variables, nullable `Boolean`s. `None` isn't sent, so it's `true`.
    defer_variables: BTreeMap<String, Option<bool>>,
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
    defers: usize,
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

impl<'a> QueryGenerator<'a> {
    pub fn new(schema: &'a SchemaDocument, seed: u64, config: GeneratorConfig) -> Self {
        Self {
            schema,
            rng: StdRng::seed_from_u64(seed),
            config,
            fragments: Vec::new(),
            variable_defs: BTreeMap::new(),
            variables: BTreeMap::new(),
            defer_variables: BTreeMap::new(),
            counters: Counters::default(),
            features: FeatureCoverage::default(),
        }
    }

    pub fn generate(mut self) -> QueryCase {
        let operation_name = "GeneratedQuery".to_string();
        let root = self
            .schema
            .query_type_name()
            .expect("schema must have a query type")
            .to_string();
        let selections = self.selection_set_for_type(&root, 0, SelectionContext::Root);

        let document = self.render(&operation_name, selections.clone(), true);
        let variables_json = self.render_variables_json(true);
        let plain_document = self.render(&operation_name, selections, false);
        let plain_variables_json = self.render_variables_json(false);

        QueryCase {
            document,
            operation_name,
            variables_json,
            plain_document,
            plain_variables_json,
            features: self.features,
        }
    }

    /// The operation and its fragments, without `@defer` and its variables when `defer` is off.
    fn render(&self, operation_name: &str, mut selections: Vec<Selection>, defer: bool) -> String {
        let mut fragments = self.fragments.clone();
        if !defer {
            strip_defer(&mut selections);
            for fragment in &mut fragments {
                strip_defer(&mut fragment.selection_set.items);
            }
        }

        let mut query_vars = Vec::new();
        for def in self.variable_defs.values() {
            query_vars.push(VariableDefinition {
                position: Pos::default(),
                name: def.name.clone(),
                var_type: q::Type::NonNullType(Box::new(q::Type::NamedType("Boolean".to_string()))),
                default_value: def.default_value.map(QueryValue::Boolean),
            });
        }
        if defer {
            for name in self.defer_variables.keys() {
                query_vars.push(VariableDefinition {
                    position: Pos::default(),
                    name: name.clone(),
                    var_type: q::Type::NamedType("Boolean".to_string()),
                    default_value: None,
                });
            }
        }

        let mut defs = vec![q::Definition::Operation(OperationDefinition::Query(
            q::Query {
                position: Pos::default(),
                name: Some(operation_name.to_string()),
                variable_definitions: query_vars,
                directives: Vec::new(),
                selection_set: SelectionSet {
                    span: (Pos::default(), Pos::default()),
                    items: selections,
                },
            },
        ))];
        defs.extend(fragments.into_iter().map(q::Definition::Fragment));

        QueryDocument { definitions: defs }.format(&Style::default())
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

        let mut directives = self.maybe_directives();
        directives.extend(self.maybe_defer());
        Some(InlineFragment {
            position: Pos::default(),
            type_condition: type_condition.map(q::TypeCondition::On),
            directives,
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

    /// `@defer`, maybe with `if` (a variable, sent or not, or a literal) and a label.
    fn maybe_defer(&mut self) -> Option<QueryDirective> {
        // no draw at all when it's off, so the other choices stay the same
        if self.config.defer_probability <= 0.0
            || self.counters.defers >= self.config.max_defers
            || !self.rng.random_bool(self.config.defer_probability)
        {
            return None;
        }
        self.counters.defers += 1;
        self.features.defers += 1;

        let mut arguments = Vec::new();
        match self.rng.random_range(0..4) {
            0 | 1 => {
                let name = format!("d{}", self.counters.defers);
                let value = match self.rng.random_range(0..3) {
                    0 => None,
                    n => Some(n == 1),
                };
                self.defer_variables.insert(name.clone(), value);
                arguments.push(("if".to_string(), QueryValue::Variable(name)));
            }
            2 => arguments.push((
                "if".to_string(),
                QueryValue::Boolean(self.rng.random_bool(0.5)),
            )),
            _ => {}
        }
        if self.rng.random_bool(0.3) {
            let label = format!("l{}", self.counters.defers);
            arguments.push(("label".to_string(), QueryValue::String(label)));
        }

        Some(QueryDirective {
            position: Pos::default(),
            name: "defer".to_string(),
            arguments,
        })
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

    fn render_variables_json(&self, defer: bool) -> String {
        let defer_values = self
            .defer_variables
            .iter()
            .filter(|_| defer)
            .filter_map(|(key, value)| value.map(|value| (key, value)));
        let body = self
            .variables
            .iter()
            .map(|(key, value)| (key, *value))
            .chain(defer_values)
            .map(|(key, value)| format!("\"{}\": {}", key, value))
            .collect::<Vec<_>>()
            .join(", ");

        format!("{{{}}}", body)
    }
}

/// Takes `@defer` off every inline fragment and fragment spread.
fn strip_defer(items: &mut [Selection]) {
    for item in items {
        match item {
            Selection::Field(field) => strip_defer(&mut field.selection_set.items),
            Selection::InlineFragment(fragment) => {
                fragment
                    .directives
                    .retain(|directive| directive.name != "defer");
                strip_defer(&mut fragment.selection_set.items);
            }
            Selection::FragmentSpread(spread) => {
                spread
                    .directives
                    .retain(|directive| directive.name != "defer");
            }
        }
    }
}

/// Puts an incremental (`@defer`) response back together the way a client would, from its parts
/// in the `incrementalSpec=v0.2` format: `pending` says where a part goes, `incremental` brings
/// its data. Gives `{"data": .., "errors": [..]}`, `errors` left out when there are none.
pub fn merge_incremental(parts: &[serde_json::Value]) -> serde_json::Value {
    use serde_json::Value;

    let mut data = parts
        .first()
        .and_then(|part| part.get("data"))
        .cloned()
        .unwrap_or(Value::Null);
    let mut errors = Vec::new();
    let mut paths: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let list = |value: &Value, key: &str| value.get(key).and_then(Value::as_array).cloned();

    for part in parts {
        errors.extend(list(part, "errors").unwrap_or_default());
        for pending in list(part, "pending").unwrap_or_default() {
            let id = pending["id"].as_str().unwrap_or_default().to_string();
            paths.insert(id, list(&pending, "path").unwrap_or_default());
        }
        for item in list(part, "incremental").unwrap_or_default() {
            errors.extend(list(&item, "errors").unwrap_or_default());
            let Some(mut path) = item["id"].as_str().and_then(|id| paths.get(id)).cloned() else {
                continue;
            };
            path.extend(list(&item, "subPath").unwrap_or_default());
            if let (Some(target), Some(new)) = (value_at(&mut data, &path), item.get("data")) {
                deep_merge(target, new);
            }
        }
        for completed in list(part, "completed").unwrap_or_default() {
            errors.extend(list(&completed, "errors").unwrap_or_default());
        }
    }

    // a response that never got to run, like one that failed validation, has no `data`
    let mut response = serde_json::json!({});
    if parts.first().is_some_and(|part| part.get("data").is_some()) {
        response["data"] = data;
    }
    if !errors.is_empty() {
        response["errors"] = Value::Array(errors);
    }
    response
}

fn value_at<'v>(
    value: &'v mut serde_json::Value,
    path: &[serde_json::Value],
) -> Option<&'v mut serde_json::Value> {
    let Some((first, rest)) = path.split_first() else {
        return Some(value);
    };
    let next = match first {
        serde_json::Value::String(key) => value.get_mut(key.as_str())?,
        serde_json::Value::Number(index) => value.get_mut(index.as_u64()? as usize)?,
        _ => return None,
    };
    value_at(next, rest)
}

fn deep_merge(target: &mut serde_json::Value, new: &serde_json::Value) {
    use serde_json::Value;
    match (target, new) {
        (Value::Object(target), Value::Object(new)) => {
            for (key, value) in new {
                match target.get_mut(key) {
                    Some(existing) => deep_merge(existing, value),
                    None => {
                        target.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (Value::Array(target), Value::Array(new)) if target.len() == new.len() => {
            for (existing, value) in target.iter_mut().zip(new) {
                deep_merge(existing, value);
            }
        }
        (target, new) => *target = new.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::{merge_incremental, GeneratorConfig, QueryGenerator};
    use graphql_tools::parser::schema::parse_schema;
    use serde_json::json;

    #[test]
    fn merges_parts_where_pending_says() {
        let parts = [
            json!({"data": {"me": {"id": "1", "reviews": [{"id": "1"}]}}, "pending": [{"id": "0", "path": ["me"]}], "hasNext": true}),
            json!({"pending": [{"id": "1", "path": ["me", "reviews", 0]}], "incremental": [{"id": "0", "data": {"name": "Uri"}}], "completed": [{"id": "0"}], "hasNext": true}),
            json!({"incremental": [{"id": "1", "subPath": ["author"], "data": {"name": "Uri"}}], "completed": [{"id": "1", "errors": [{"message": "x"}]}], "hasNext": false}),
        ];
        assert_eq!(
            merge_incremental(&parts),
            json!({
                "data": {"me": {"id": "1", "name": "Uri", "reviews": [{"id": "1"}]}},
                "errors": [{"message": "x"}],
            })
        );
    }

    #[test]
    fn plain_document_has_no_defer() {
        let schema = parse_schema::<String>("type Query { a: A } type A { b: String c: A }")
            .unwrap()
            .into_static();
        let config = GeneratorConfig {
            defer_probability: 1.0,
            inline_fragment_probability: 1.0,
            ..Default::default()
        };
        let case = (0..20)
            .map(|seed| QueryGenerator::new(&schema, seed, config.clone()).generate())
            .find(|case| case.features.defers > 0)
            .expect("a case with @defer");
        assert!(case.document.contains("@defer"));
        assert!(!case.plain_document.contains("@defer"));
        assert!(!case.plain_document.contains("$d"));
    }
}
