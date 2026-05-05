use rand::{prelude::IndexedRandom, rngs::StdRng, seq::SliceRandom, RngExt, SeedableRng};
use std::collections::BTreeMap;

/// A minimal schema abstraction for schema-aware query generation.
///
/// Keep this trait small and adapt your real schema representation to it.
/// For Hive Router you can implement this over whatever validated schema model
/// you already have instead of coupling the generator to a specific parser crate.
pub trait SchemaView {
    fn query_type(&self) -> &str;

    fn type_kind(&self, type_name: &str) -> Option<TypeKind>;

    /// Fields visible directly from this type.
    ///
    /// For objects: object fields.
    /// For interfaces: interface fields.
    /// For unions: usually empty; `__typename` is generated separately.
    fn fields(&self, type_name: &str) -> Vec<FieldDef>;

    /// Possible concrete object types for an object/interface/union.
    ///
    /// For objects, returning itself is convenient.
    fn possible_types(&self, type_name: &str) -> Vec<String>;

    /// Values of an enum type. Used for argument generation.
    fn enum_values(&self, _type_name: &str) -> Vec<String> {
        Vec::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeKind {
    Object,
    Interface,
    Union,
    Scalar,
    Enum,
    InputObject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDef {
    pub name: String,
    pub ty: TypeRef,
    pub args: Vec<InputValueDef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputValueDef {
    pub name: String,
    pub ty: TypeRef,
    pub default_value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeRef {
    Named(String),
    List(Box<TypeRef>),
    NonNull(Box<TypeRef>),
}

impl TypeRef {
    pub fn named_type(&self) -> &str {
        match self {
            TypeRef::Named(name) => name,
            TypeRef::List(inner) | TypeRef::NonNull(inner) => inner.named_type(),
        }
    }

    pub fn is_non_null(&self) -> bool {
        matches!(self, TypeRef::NonNull(_))
    }

    pub fn as_graphql(&self) -> String {
        match self {
            TypeRef::Named(name) => name.clone(),
            TypeRef::List(inner) => format!("[{}]", inner.as_graphql()),
            TypeRef::NonNull(inner) => format!("{}!", inner.as_graphql()),
        }
    }
}

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
    /// JSON object string. Kept as a string to avoid forcing a serde_json dependency here.
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

pub struct QueryGenerator<'a, S: SchemaView> {
    schema: &'a S,
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

impl<'a, S: SchemaView> QueryGenerator<'a, S> {
    pub fn new(schema: &'a S, seed: u64, config: GeneratorConfig) -> Self {
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
        let root = self.schema.query_type().to_string();
        let selections = self.selection_set_for_type(&root, 0, SelectionContext::Root);

        let variable_defs = self.render_variable_defs();
        let variables_json = self.render_variables_json();

        let mut document = String::new();
        document.push_str("query ");
        document.push_str(&operation_name);
        document.push_str(&variable_defs);
        document.push_str(" ");
        render_selection_set(&mut document, &selections, 0);

        for fragment in &self.fragments {
            document.push_str("\n\n");
            render_fragment_definition(&mut document, fragment);
        }

        QueryCase {
            document,
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

        let kind = self.schema.type_kind(type_name).unwrap_or(TypeKind::Object);
        let mut selections = Vec::new();

        if matches!(
            kind,
            TypeKind::Object | TypeKind::Interface | TypeKind::Union
        ) {
            if self.rng.random_bool(0.55) || matches!(kind, TypeKind::Union) {
                selections.push(Selection::Field(FieldSelection {
                    alias: None,
                    name: "__typename".to_string(),
                    args: Vec::new(),
                    directives: self.maybe_directives(),
                    selection_set: Vec::new(),
                }));
            }
        }

        if !matches!(kind, TypeKind::Union) {
            let mut fields = self.schema.fields(type_name);
            fields.shuffle(&mut self.rng);

            let width = self.rng.random_range(1..=self.config.max_width.max(1));
            for field in fields.into_iter().take(width) {
                selections.push(self.field_selection(field.clone(), depth));

                if self
                    .rng
                    .random_bool(self.config.duplicate_field_probability)
                {
                    selections.push(self.field_selection(field, depth));
                    self.features.duplicated_response_keys += 1;
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
            selections.push(Selection::Field(FieldSelection {
                alias: None,
                name: "__typename".to_string(),
                args: Vec::new(),
                directives: Vec::new(),
                selection_set: Vec::new(),
            }));
        }

        selections.shuffle(&mut self.rng);
        selections
    }

    fn leafish_selection_set(&mut self, type_name: &str) -> Vec<Selection> {
        let kind = self.schema.type_kind(type_name).unwrap_or(TypeKind::Object);

        if matches!(kind, TypeKind::Union) {
            return vec![Selection::Field(FieldSelection {
                alias: None,
                name: "__typename".to_string(),
                args: Vec::new(),
                directives: self.maybe_directives(),
                selection_set: Vec::new(),
            })];
        }

        let mut selections = Vec::new();

        if self.rng.random_bool(0.60) {
            selections.push(Selection::Field(FieldSelection {
                alias: None,
                name: "__typename".to_string(),
                args: Vec::new(),
                directives: self.maybe_directives(),
                selection_set: Vec::new(),
            }));
        }

        let mut scalar_fields = self
            .schema
            .fields(type_name)
            .into_iter()
            .filter(|field| self.is_leaf_output_type(&field.ty))
            .collect::<Vec<_>>();
        scalar_fields.shuffle(&mut self.rng);

        for field in scalar_fields
            .into_iter()
            .take(self.config.max_width.min(3).max(1))
        {
            selections.push(self.field_selection(field, self.config.max_depth));
        }

        if selections.is_empty() {
            selections.push(Selection::Field(FieldSelection {
                alias: None,
                name: "__typename".to_string(),
                args: Vec::new(),
                directives: Vec::new(),
                selection_set: Vec::new(),
            }));
        }

        selections
    }

    fn field_selection(&mut self, field: FieldDef, depth: usize) -> Selection {
        let named = field.ty.named_type().to_string();
        let kind = self.schema.type_kind(&named).unwrap_or(TypeKind::Scalar);
        let needs_selection = matches!(
            kind,
            TypeKind::Object | TypeKind::Interface | TypeKind::Union
        );

        let alias = if self.rng.random_bool(self.config.alias_probability) {
            self.features.aliases += 1;
            self.counters.alias += 1;
            Some(format!("a{}_{}", self.counters.alias, field.name))
        } else {
            None
        };

        let args = self.args_for_field(&field);
        let directives = self.maybe_directives();
        let selection_set = if needs_selection {
            self.selection_set_for_type(&named, depth + 1, SelectionContext::Field)
        } else {
            Vec::new()
        };

        Selection::Field(FieldSelection {
            alias,
            name: field.name,
            args,
            directives,
            selection_set,
        })
    }

    fn args_for_field(&mut self, field: &FieldDef) -> Vec<(String, String)> {
        let mut args = Vec::new();

        for arg in &field.args {
            let required = arg.ty.is_non_null() && arg.default_value.is_none();

            if required || self.rng.random_bool(0.35) {
                if let Some(value) = self.literal_for_input_type(&arg.ty) {
                    args.push((arg.name.clone(), value));
                }
            }
        }

        args
    }

    fn literal_for_input_type(&mut self, ty: &TypeRef) -> Option<String> {
        match ty {
            TypeRef::NonNull(inner) => self.literal_for_input_type(inner),
            TypeRef::List(inner) => {
                let len = self.rng.random_range(0..=3);
                let mut values = Vec::new();
                for _ in 0..len {
                    values.push(self.literal_for_input_type(inner)?);
                }
                Some(format!("[{}]", values.join(", ")))
            }
            TypeRef::Named(name) => match name.as_str() {
                "ID" => Some(format!("\"id-{}\"", self.rng.random_range(0..1000))),
                "String" => Some(format!("\"s{}\"", self.rng.random_range(0..1000))),
                "Int" => Some(self.rng.random_range(0..100).to_string()),
                "Float" => Some(format!("{}.5", self.rng.random_range(0..100))),
                "Boolean" => Some(self.rng.random_bool(0.5).to_string()),
                other => match self.schema.type_kind(other) {
                    Some(TypeKind::Enum) => self
                        .schema
                        .enum_values(other)
                        .choose(&mut self.rng)
                        .cloned(),
                    // Input objects are intentionally skipped in this starter implementation.
                    // Add recursive object literal generation once you need it.
                    _ => None,
                },
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

    /// `rand::Rng::gen_bool` needs `&mut self`; these helpers are split so callers that only
    /// need a cheap guard can be kept readable. The actual random decision is re-made inside
    /// the constructor functions when necessary.
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
            name: name.clone(),
            type_condition,
            directives: Vec::new(),
            selection_set: fragment_selection_set,
        });
        self.features.named_fragments += 1;

        Some(FragmentSpread {
            name,
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

        let scoped_type = type_condition.as_deref().unwrap_or(current_type);
        let selection_set =
            self.selection_set_for_type(scoped_type, depth + 1, SelectionContext::InlineFragment);

        Some(InlineFragment {
            type_condition,
            directives: self.maybe_directives(),
            selection_set,
        })
    }

    fn compatible_type_condition(&mut self, current_type: &str) -> Option<String> {
        let current_kind = self.schema.type_kind(current_type)?;

        match current_kind {
            TypeKind::Object => Some(current_type.to_string()),
            TypeKind::Interface | TypeKind::Union => {
                let mut candidates = self.schema.possible_types(current_type);

                // Keep the abstract type itself as a candidate for interfaces.
                // Union fragments must target object/interface/union, but selecting fields
                // directly on a union still needs concrete fragments to be useful.
                if matches!(current_kind, TypeKind::Interface) {
                    candidates.push(current_type.to_string());
                }

                candidates.sort();
                candidates.dedup();
                candidates.choose(&mut self.rng).cloned()
            }
            _ => None,
        }
    }

    fn record_type_condition_feature(&mut self, type_condition: &str) {
        match self.schema.type_kind(type_condition) {
            Some(TypeKind::Interface | TypeKind::Union) => {
                self.features.abstract_type_conditions += 1
            }
            Some(TypeKind::Object) => self.features.concrete_type_conditions += 1,
            _ => {}
        }
    }

    fn maybe_directives(&mut self) -> Vec<Directive> {
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

    fn directive(&mut self, name: &'static str) -> Directive {
        self.counters.directives += 1;

        match name {
            "skip" => self.features.skip_directives += 1,
            "include" => self.features.include_directives += 1,
            _ => {}
        }

        let value = match name {
            // Bias toward selections staying visible, but still produce false/invisible branches.
            "skip" => self.rng.random_bool(0.20),
            "include" => self.rng.random_bool(0.80),
            _ => self.rng.random_bool(0.50),
        };

        let arg = if self
            .rng
            .random_bool(self.config.variable_directive_probability)
        {
            self.features.directive_variables += 1;
            BoolValue::Variable(self.bool_variable(value))
        } else {
            BoolValue::Literal(value)
        };

        Directive {
            name: name.to_string(),
            arg,
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
                ty: "Boolean!".to_string(),
                default_value: with_default.then_some(value),
            },
        );

        if !omit_from_variables {
            self.variables.insert(name.clone(), value);
        }

        name
    }

    fn is_leaf_output_type(&self, ty: &TypeRef) -> bool {
        let named = ty.named_type();
        matches!(
            self.schema.type_kind(named),
            Some(TypeKind::Scalar | TypeKind::Enum)
        )
    }

    fn render_variable_defs(&self) -> String {
        if self.variable_defs.is_empty() {
            return String::new();
        }

        let defs = self
            .variable_defs
            .values()
            .map(|def| match def.default_value {
                Some(value) => format!("${}: {} = {}", def.name, def.ty, value),
                None => format!("${}: {}", def.name, def.ty),
            })
            .collect::<Vec<_>>()
            .join(", ");

        format!("({})", defs)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionContext {
    Root,
    Field,
    FragmentDefinition,
    InlineFragment,
}

#[derive(Debug, Clone)]
struct VariableDef {
    name: String,
    ty: String,
    default_value: Option<bool>,
}

#[derive(Debug, Clone)]
enum Selection {
    Field(FieldSelection),
    FragmentSpread(FragmentSpread),
    InlineFragment(InlineFragment),
}

#[derive(Debug, Clone)]
struct FieldSelection {
    alias: Option<String>,
    name: String,
    args: Vec<(String, String)>,
    directives: Vec<Directive>,
    selection_set: Vec<Selection>,
}

#[derive(Debug, Clone)]
struct FragmentSpread {
    name: String,
    directives: Vec<Directive>,
}

#[derive(Debug, Clone)]
struct InlineFragment {
    type_condition: Option<String>,
    directives: Vec<Directive>,
    selection_set: Vec<Selection>,
}

#[derive(Debug, Clone)]
struct FragmentDefinition {
    name: String,
    type_condition: String,
    directives: Vec<Directive>,
    selection_set: Vec<Selection>,
}

#[derive(Debug, Clone)]
struct Directive {
    name: String,
    arg: BoolValue,
}

#[derive(Debug, Clone)]
enum BoolValue {
    Literal(bool),
    Variable(String),
}

fn render_selection_set(out: &mut String, selections: &[Selection], indent: usize) {
    out.push_str("{\n");

    for selection in selections {
        out.push_str(&"  ".repeat(indent + 1));
        render_selection(out, selection, indent + 1);
        out.push('\n');
    }

    out.push_str(&"  ".repeat(indent));
    out.push('}');
}

fn render_selection(out: &mut String, selection: &Selection, indent: usize) {
    match selection {
        Selection::Field(field) => {
            if let Some(alias) = &field.alias {
                out.push_str(alias);
                out.push_str(": ");
            }

            out.push_str(&field.name);

            if !field.args.is_empty() {
                out.push('(');
                for (index, (name, value)) in field.args.iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(name);
                    out.push_str(": ");
                    out.push_str(value);
                }
                out.push(')');
            }

            render_directives(out, &field.directives);

            if !field.selection_set.is_empty() {
                out.push(' ');
                render_selection_set(out, &field.selection_set, indent);
            }
        }
        Selection::FragmentSpread(spread) => {
            out.push_str("...");
            out.push_str(&spread.name);
            render_directives(out, &spread.directives);
        }
        Selection::InlineFragment(fragment) => {
            out.push_str("...");
            if let Some(type_condition) = &fragment.type_condition {
                out.push_str(" on ");
                out.push_str(type_condition);
            }
            render_directives(out, &fragment.directives);
            out.push(' ');
            render_selection_set(out, &fragment.selection_set, indent);
        }
    }
}

fn render_fragment_definition(out: &mut String, fragment: &FragmentDefinition) {
    out.push_str("fragment ");
    out.push_str(&fragment.name);
    out.push_str(" on ");
    out.push_str(&fragment.type_condition);
    render_directives(out, &fragment.directives);
    out.push(' ');
    render_selection_set(out, &fragment.selection_set, 0);
}

fn render_directives(out: &mut String, directives: &[Directive]) {
    for directive in directives {
        out.push(' ');
        out.push('@');
        out.push_str(&directive.name);
        out.push_str("(if: ");
        match &directive.arg {
            BoolValue::Literal(value) => out.push_str(&value.to_string()),
            BoolValue::Variable(name) => {
                out.push('$');
                out.push_str(name);
            }
        }
        out.push(')');
    }
}

// -----------------------------------------------------------------------------
// Example in-memory schema adapter for tests and initial experimentation.
// Replace this with an adapter over your real validated schema model.
// -----------------------------------------------------------------------------

#[derive(Default)]
pub struct TestSchema {
    query_type: String,
    types: BTreeMap<String, TypeInfo>,
    possible_types: BTreeMap<String, Vec<String>>,
    enum_values: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone)]
struct TypeInfo {
    kind: TypeKind,
    fields: Vec<FieldDef>,
}

impl TestSchema {
    pub fn synthetic() -> Self {
        let mut schema = Self {
            query_type: "Query".to_string(),
            ..Self::default()
        };

        schema.scalar("ID");
        schema.scalar("String");
        schema.scalar("Int");
        schema.scalar("Float");
        schema.scalar("Boolean");
        schema.enum_type("Role", &["ADMIN", "USER", "GUEST"]);

        schema.interface(
            "Node",
            vec![
                field("id", non_null(named("ID"))),
                field("name", named("String")),
            ],
        );

        schema.object(
            "User",
            vec![
                field("id", non_null(named("ID"))),
                field("name", named("String")),
                field("role", named("Role")),
                field("friend", named("User")),
                field("friends", list(non_null(named("User")))).arg("limit", named("Int")),
                field("posts", list(non_null(named("Post")))),
            ],
        );

        schema.object(
            "Post",
            vec![
                field("id", non_null(named("ID"))),
                field("name", named("String")),
                field("title", named("String")),
                field("author", named("User")),
            ],
        );

        schema.union("SearchResult", &["User", "Post"]);

        schema.object(
            "Query",
            vec![
                field("node", named("Node")).arg("id", non_null(named("ID"))),
                field("user", named("User")).arg("id", non_null(named("ID"))),
                field("search", list(non_null(named("SearchResult")))).arg("text", named("String")),
            ],
        );

        schema.possible_types.insert(
            "Node".to_string(),
            vec!["User".to_string(), "Post".to_string()],
        );
        schema.possible_types.insert(
            "SearchResult".to_string(),
            vec!["User".to_string(), "Post".to_string()],
        );
        schema
            .possible_types
            .insert("User".to_string(), vec!["User".to_string()]);
        schema
            .possible_types
            .insert("Post".to_string(), vec!["Post".to_string()]);
        schema
            .possible_types
            .insert("Query".to_string(), vec!["Query".to_string()]);

        schema
    }

    fn scalar(&mut self, name: &str) {
        self.types.insert(
            name.to_string(),
            TypeInfo {
                kind: TypeKind::Scalar,
                fields: Vec::new(),
            },
        );
    }

    fn enum_type(&mut self, name: &str, values: &[&str]) {
        self.types.insert(
            name.to_string(),
            TypeInfo {
                kind: TypeKind::Enum,
                fields: Vec::new(),
            },
        );
        self.enum_values.insert(
            name.to_string(),
            values.iter().map(|value| value.to_string()).collect(),
        );
    }

    fn object(&mut self, name: &str, fields: Vec<FieldDef>) {
        self.types.insert(
            name.to_string(),
            TypeInfo {
                kind: TypeKind::Object,
                fields,
            },
        );
    }

    fn interface(&mut self, name: &str, fields: Vec<FieldDef>) {
        self.types.insert(
            name.to_string(),
            TypeInfo {
                kind: TypeKind::Interface,
                fields,
            },
        );
    }

    fn union(&mut self, name: &str, members: &[&str]) {
        self.types.insert(
            name.to_string(),
            TypeInfo {
                kind: TypeKind::Union,
                fields: Vec::new(),
            },
        );
        self.possible_types.insert(
            name.to_string(),
            members.iter().map(|member| member.to_string()).collect(),
        );
    }
}

impl SchemaView for TestSchema {
    fn query_type(&self) -> &str {
        &self.query_type
    }

    fn type_kind(&self, type_name: &str) -> Option<TypeKind> {
        self.types.get(type_name).map(|info| info.kind)
    }

    fn fields(&self, type_name: &str) -> Vec<FieldDef> {
        self.types
            .get(type_name)
            .map(|info| info.fields.clone())
            .unwrap_or_default()
    }

    fn possible_types(&self, type_name: &str) -> Vec<String> {
        self.possible_types
            .get(type_name)
            .cloned()
            .unwrap_or_else(|| vec![type_name.to_string()])
    }

    fn enum_values(&self, type_name: &str) -> Vec<String> {
        self.enum_values.get(type_name).cloned().unwrap_or_default()
    }
}

fn field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        args: Vec::new(),
    }
}

trait FieldBuilderExt {
    fn arg(self, name: &str, ty: TypeRef) -> Self;
}

impl FieldBuilderExt for FieldDef {
    fn arg(mut self, name: &str, ty: TypeRef) -> Self {
        self.args.push(InputValueDef {
            name: name.to_string(),
            ty,
            default_value: None,
        });
        self
    }
}

fn named(name: &str) -> TypeRef {
    TypeRef::Named(name.to_string())
}

fn list(inner: TypeRef) -> TypeRef {
    TypeRef::List(Box::new(inner))
}

fn non_null(inner: TypeRef) -> TypeRef {
    TypeRef::NonNull(Box::new(inner))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_a_complex_query() {
        let schema = TestSchema::synthetic();
        let case = QueryGenerator::new(&schema, 42, GeneratorConfig::default()).generate();

        eprintln!("{}", case.document);
        eprintln!("variables: {}", case.variables_json);
        eprintln!("features: {:#?}", case.features);

        assert!(case.document.starts_with("query GeneratedQuery"));
        assert!(case.document.contains("@skip") || case.document.contains("@include"));
        assert!(case.document.contains("fragment") || case.document.contains("... on"));
    }

    #[test]
    fn same_seed_generates_same_query() {
        let schema = TestSchema::synthetic();
        let a = QueryGenerator::new(&schema, 7, GeneratorConfig::default()).generate();
        let b = QueryGenerator::new(&schema, 7, GeneratorConfig::default()).generate();

        assert_eq!(a.document, b.document);
        assert_eq!(a.variables_json, b.variables_json);
    }

    #[test]
    fn different_seeds_generate_different_queries() {
        let schema = TestSchema::synthetic();
        let a = QueryGenerator::new(&schema, 7, GeneratorConfig::default()).generate();
        let b = QueryGenerator::new(&schema, 8, GeneratorConfig::default()).generate();

        assert_ne!(a.document, b.document);
    }
}

fn main() {
    let schema = TestSchema::synthetic();

    let case = QueryGenerator::new(&schema, 42, GeneratorConfig::default()).generate();

    println!("{}", case.document);
    println!("{}", case.variables_json);
    println!("{:#?}", case.features);
}
