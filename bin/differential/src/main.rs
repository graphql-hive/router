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

        for frag in self.fragments.into_iter() {
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
        let body = self
            .variables
            .iter()
            .map(|(key, value)| format!("\"{}\": {}", key, value))
            .collect::<Vec<_>>()
            .join(", ");

        format!("{{{}}}", body)
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

    println!("Running {} differential queries against:", num_queries);
    println!("  baseline: {}", baseline_endpoint);
    println!("  candidate: {}", candidate_endpoint);
    println!("--------------------------------------------------");

    for i in 0..num_queries {
        let seed = i as u64 + 42;
        let case = QueryGenerator::new(&schema, seed, GeneratorConfig::default()).generate();

        println!("Query #{}:", i + 1);

        let res1_future = execute_query(
            &client,
            baseline_endpoint,
            &case.document,
            &case.variables_json,
        );
        let res2_future = execute_query(
            &client,
            candidate_endpoint,
            &case.document,
            &case.variables_json,
        );

        let (res1, res2) = tokio::join!(res1_future, res2_future);

        match (res1, res2) {
            (Ok(res1), Ok(res2)) => {
                // The comparison should happen on `result.data` and not `result.errors`.
                // The errors part may differ, so we should just check wether errors are present or not.
                let (data1, _errors1) = match &res1 {
                    JsonValue::Object(res) => {
                        (res.get("data").cloned(), res.get("errors").cloned())
                    }
                    _ => panic!("Not a graphql response"),
                };
                let (data2, _errors2) = match &res2 {
                    JsonValue::Object(res) => {
                        (res.get("data").cloned(), res.get("errors").cloned())
                    }
                    _ => panic!("Not a graphql response"),
                };

                if data1 != data2 {
                    differences += 1;

                    let dir = format!("./failed-tests/case-{}", i);
                    std::fs::create_dir_all(&dir).expect("to create a directory");
                    std::fs::write(format!("{}/query.graphql", dir), case.document.clone())
                        .expect("to create query.graphql");
                    std::fs::write(
                        format!("{}/variables.json", dir),
                        case.variables_json.clone(),
                    )
                    .expect("to create query.graphql");
                    std::fs::write(
                        format!("{}/endpoint-1.json", dir),
                        serde_json::to_string_pretty(&res1).unwrap(),
                    )
                    .expect("to create endpoint-1.json");
                    std::fs::write(
                        format!("{}/endpoint-2.json", dir),
                        serde_json::to_string_pretty(&res2).unwrap(),
                    )
                    .expect("to create endpoint-2.json");

                    println!("⚠️ Responses differ");
                } else {
                    println!("✅ Responses match");
                }
            }
            (Err(e1), Err(e2)) => {
                println!("⚠️ Both endpoints failed");
                println!("  baseline: {}", e1);
                println!("  candidate: {}", e2);
            }
            (Err(e1), Ok(_)) => {
                println!("❌ Baseline endpoint failed: {}", e1);
                differences += 1;
            }
            (Ok(_), Err(e2)) => {
                println!("❌ Candidate endpoint failed: {}", e2);
                differences += 1;
            }
        }
        println!("--------------------------------------------------");
    }

    if differences == 0 {
        println!("🎉 All queries returned matching results!");
    } else {
        println!("⚠️ Found {} queries with different results.", differences);
        std::process::exit(1);
    }
}
