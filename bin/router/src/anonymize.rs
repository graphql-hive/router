//! Anonymizes a supergraph and its operations with a secret key.
//!
//! Every name and string goes through BLAKE3 in keyed mode, so the same key always gives the
//! same output. That's what lets operations be anonymized on their own, without the
//! supergraph and without a mapping file. Without the key there's no way to run a dictionary
//! attack against the output, with the key there is, so treat it like a password.
//!
//! Names are mapped by kind (type, field, argument...), never by coordinate.
//! `User.id` and `Node.id` both become the same `f_...`, which keeps interfaces valid
//! and means operations need no schema to be anonymized.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Write;
use std::path::Path;

use bumpalo::Bump;
use graphql_tools::parser::query::{
    Definition as OperationDocDefinition, Directive, Number, OperationDefinition, Selection,
    SelectionSet, Type, TypeCondition, Value, VariableDefinition,
};
use graphql_tools::parser::schema::{
    parse_schema_definitions, Definition, Field, InputValue, TypeDefinition,
};
use graphql_tools::parser::{parse_query, Style};
use rand::TryRng;

const KEY_ENV: &str = "HIVE_ANON_KEY";

const SPEC_URL: &str = "https://specs.apollo.dev/";

const KEPT_TYPES: &[&str] = &[
    "String",
    "Int",
    "Float",
    "Boolean",
    "ID",
    "Query",
    "Mutation",
    "Subscription",
];

// types and directives namespaced by a spec, `join__Graph`, `link__Import` and friends
const SPEC_PREFIXES: &[&str] = &[
    "join__",
    "link__",
    "core__",
    "context__",
    "federation__",
    "inaccessible__",
    "tag__",
    "authenticated__",
    "requiresScopes__",
    "policy__",
    "cost__",
    "listSize__",
];

const KEPT_DIRECTIVES: &[&str] = &[
    "skip",
    "include",
    "deprecated",
    "specifiedBy",
    "oneOf",
    "defer",
    "stream",
    "link",
    "core",
    "inaccessible",
    "tag",
    "authenticated",
    "requiresScopes",
    "policy",
    "context",
    "cost",
    "listSize",
];

const USAGE: &str = "\
Usage:
  hive_router anonymize keygen
  hive_router anonymize supergraph [--key-file <path>] <supergraph.graphql>
  hive_router anonymize operations [--key-file <path>] <operation.graphql>
  hive_router anonymize operations [--key-file <path>] <input dir> <output dir>

The key comes from --key-file or the HIVE_ANON_KEY env var, 64 hex characters made by `keygen`.
Use the same key for the supergraph and its operations.";

#[derive(Clone, Copy)]
enum Kind {
    Type,
    Field,
    Argument,
    InputField,
    EnumValue,
    Directive,
    Subgraph,
    Context,
    Variable,
    Operation,
    Fragment,
    String,
    Number,
}

impl Kind {
    // doubles as the domain separator in the hash, so every kind needs its own
    fn prefix(self) -> &'static str {
        match self {
            Kind::Type => "T",
            Kind::Field => "f",
            Kind::Argument => "a",
            Kind::InputField => "i",
            Kind::EnumValue => "E",
            Kind::Directive => "d",
            Kind::Subgraph => "g",
            Kind::Context => "c",
            Kind::Variable => "v",
            Kind::Operation => "O",
            Kind::Fragment => "F",
            Kind::String => "s",
            Kind::Number => "n",
        }
    }
}

fn is_spec(name: &str) -> bool {
    name.starts_with("__") || SPEC_PREFIXES.iter().any(|prefix| name.starts_with(prefix))
}

fn is_kept_directive(name: &str) -> bool {
    KEPT_DIRECTIVES.contains(&name) || is_spec(name)
}

pub struct Anonymizer {
    key: [u8; 32],
}

impl Anonymizer {
    pub fn new(key: [u8; 32]) -> Self {
        Self { key }
    }

    pub fn from_hex(hex: &str) -> Result<Self, String> {
        let hex = hex.trim();
        if hex.len() != 64 || !hex.is_ascii() {
            return Err("the key has to be 64 hex characters (256 bits)".into());
        }
        let mut key = [0u8; 32];
        for (i, byte) in key.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
                .map_err(|_| "the key has to be 64 hex characters (256 bits)")?;
        }
        Ok(Self::new(key))
    }

    /// 256 bits from the OS random source, hex encoded.
    pub fn generate_key() -> Result<String, String> {
        let mut key = [0u8; 32];
        rand::rngs::SysRng
            .try_fill_bytes(&mut key)
            .map_err(|err| format!("failed to read OS randomness: {err}"))?;
        Ok(to_hex(&key))
    }

    pub fn anonymize_supergraph(&self, sdl: &str) -> Result<String, String> {
        let mut output = Vec::with_capacity(sdl.len());
        self.anonymize_supergraph_to(sdl, &mut output)?;
        String::from_utf8(output).map_err(|err| err.to_string())
    }

    /// Writes each definition out as soon as it's done, so the anonymized supergraph is never
    /// in memory as a whole.
    pub fn anonymize_supergraph_to(
        &self,
        sdl: &str,
        output: &mut impl std::io::Write,
    ) -> Result<(), String> {
        let arena = Bump::new();
        Run::new(&self.key, &arena).supergraph(sdl, output)
    }

    pub fn anonymize_operation(&self, source: &str) -> Result<String, String> {
        let arena = Bump::new();
        Run::new(&self.key, &arena).operation_document(source)
    }

    fn name(&self, kind: Kind, input: &str) -> String {
        let arena = Bump::new();
        Run::new(&self.key, &arena).name(kind, input).to_string()
    }
}

// one document's worth of work. Names live in the arena, so the AST borrows them instead of
// owning a `String` each, and it all goes away in one go when the run is done
struct Run<'a> {
    key: &'a [u8; 32],
    arena: &'a Bump,
    // input -> output for each kind, names repeat a lot so this skips most of the hashing
    cache: RefCell<[HashMap<&'a str, &'a str>; Kind::Number as usize + 1]>,
    // output -> input, to catch a collision instead of silently merging two names
    seen: RefCell<HashMap<&'a str, &'a str>>,
    // `key: "id"` shows up on every entity in every subgraph, parse each field set only once
    field_sets: RefCell<HashMap<&'a str, &'a str>>,
}

impl<'a> Run<'a> {
    fn new(key: &'a [u8; 32], arena: &'a Bump) -> Self {
        Self {
            key,
            arena,
            cache: RefCell::default(),
            seen: RefCell::default(),
            field_sets: RefCell::default(),
        }
    }

    fn digest(&self, kind: Kind, input: &str) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new_keyed(self.key);
        hasher.update(kind.prefix().as_bytes());
        hasher.update(&[0]);
        hasher.update(input.as_bytes());
        *hasher.finalize().as_bytes()
    }

    fn name(&self, kind: Kind, input: &str) -> &'a str {
        if let Some(output) = self.cache.borrow()[kind as usize].get(input) {
            return output;
        }
        // 64 bits, collisions are astronomically unlikely, but checked anyway
        let digest = self.digest(kind, input);
        let mut output = bumpalo::collections::String::with_capacity_in(18, self.arena);
        output.push_str(kind.prefix());
        output.push('_');
        for byte in &digest[..8] {
            let _ = write!(output, "{byte:02x}");
        }
        let output = output.into_bump_str();
        let input = &*self.arena.alloc_str(input);
        if let Some(previous) = self.seen.borrow_mut().insert(output, input) {
            assert!(
                previous == input,
                "hash collision between {previous:?} and {input:?}, generate another key"
            );
        }
        self.cache.borrow_mut()[kind as usize].insert(input, output);
        output
    }

    // kept names come back as they are, so these work on strings that aren't from the arena too
    fn type_name<'n>(&self, name: &'n str) -> &'n str
    where
        'a: 'n,
    {
        if KEPT_TYPES.contains(&name) || is_spec(name) {
            name
        } else {
            self.name(Kind::Type, name)
        }
    }

    fn field_name<'n>(&self, name: &'n str) -> &'n str
    where
        'a: 'n,
    {
        if name.starts_with("__") {
            name
        } else {
            self.name(Kind::Field, name)
        }
    }

    fn directive_name<'n>(&self, name: &'n str) -> &'n str
    where
        'a: 'n,
    {
        if is_kept_directive(name) {
            name
        } else {
            self.name(Kind::Directive, name)
        }
    }

    // `@link` imports are `"@directive"` or a type name
    fn import_name(&self, name: &str) -> String {
        match name.strip_prefix('@') {
            Some(directive) => format!("@{}", self.directive_name(directive)),
            None => self.type_name(name).to_string(),
        }
    }

    fn number(&self, input: impl std::fmt::Display) -> u32 {
        // digits fit on the stack, no need for a `String` per number
        let mut buf = [0u8; 64];
        let mut cursor = std::io::Cursor::new(&mut buf[..]);
        let _ = std::io::Write::write_fmt(&mut cursor, format_args!("{input}"));
        let len = cursor.position() as usize;
        let digits = std::str::from_utf8(&buf[..len]).unwrap_or_default();
        let digest = self.digest(Kind::Number, digits);
        u32::from_le_bytes([digest[0], digest[1], digest[2], digest[3]])
    }

    // string values are always owned by the parser, that's the one `String` left per value
    fn value(&self, value: &mut Value<'a, &'a str>) {
        match value {
            Value::Variable(name) => *name = self.name(Kind::Variable, name),
            Value::Int(number) => {
                let input = number.as_i64().unwrap_or_default();
                *number = Number::from((self.number(input) >> 1) as i32);
            }
            // the .5 keeps it a float when printed
            Value::Float(float) => *float = (self.number(*float) % 1_000_000) as f64 + 0.5,
            Value::String(string) => {
                let name = self.name(Kind::String, string);
                rewrite(string, name);
            }
            Value::Enum(name) => *name = self.name(Kind::EnumValue, name),
            Value::List(items) => items.iter_mut().for_each(|item| self.value(item)),
            Value::Object(fields) => {
                for (name, value) in fields {
                    *name = self.name(Kind::InputField, name);
                    self.value(value);
                }
            }
            Value::Boolean(_) | Value::Null => {}
        }
    }

    fn arguments(&self, arguments: &mut [(&'a str, Value<'a, &'a str>)]) {
        for (name, value) in arguments {
            *name = self.name(Kind::Argument, name);
            self.value(value);
        }
    }

    fn type_ref(&self, ty: &mut Type<'a, &'a str>) {
        match ty {
            Type::NamedType(name) => *name = self.type_name(name),
            Type::ListType(inner) | Type::NonNullType(inner) => self.type_ref(inner),
        }
    }

    // `[User!]!` as a string, like in `@join__field(type:)`, renamed in place
    fn type_string(&self, ty: &mut String) {
        let name = ty.trim_matches(|c: char| matches!(c, '[' | ']' | '!') || c.is_whitespace());
        if name.is_empty() || KEPT_TYPES.contains(&name) || is_spec(name) {
            return;
        }
        let start = name.as_ptr() as usize - ty.as_ptr() as usize;
        let range = start..start + name.len();
        let name = self.name(Kind::Type, name);
        ty.replace_range(range, name);
    }

    // `id organization { id }` as a string, like in `@join__type(key:)`
    fn field_set(&self, fields: &str) -> &'a str {
        let trimmed = fields.trim();
        if let Some(output) = self.field_sets.borrow().get(trimmed) {
            return output;
        }
        // context selections come wrapped in braces already, keys and requires don't
        let braced = trimmed.starts_with('{');
        // parsed from an arena copy, so its AST can borrow the same way the document's does
        let source: &'a str = if braced {
            self.arena.alloc_str(trimmed)
        } else {
            bumpalo::format!(in self.arena, "{{{}}}", trimmed).into_bump_str()
        };
        let mut document = parse_query::<&'a str>(source)
            .unwrap_or_else(|err| panic!("invalid field set {fields:?}: {err}"));
        let Some(OperationDocDefinition::Operation(OperationDefinition::SelectionSet(set))) =
            document.definitions.first_mut()
        else {
            panic!("invalid field set {fields:?}");
        };
        self.selection_set(set);
        let printed = document.format(&Style::default());
        // strings are hashed by now, so there's no whitespace inside a value to break
        let mut output = bumpalo::collections::String::with_capacity_in(printed.len(), self.arena);
        for word in printed.split_whitespace() {
            if !output.is_empty() {
                output.push(' ');
            }
            output.push_str(word);
        }
        let output = output.into_bump_str();
        let output = if braced {
            output
        } else {
            output
                .strip_prefix("{ ")
                .and_then(|inner| inner.strip_suffix(" }"))
                .unwrap_or(output)
        };
        let key = if braced {
            source
        } else {
            &source[1..source.len() - 1]
        };
        self.field_sets.borrow_mut().insert(key, output);
        output
    }

    fn selection_set(&self, set: &mut SelectionSet<'a, &'a str>) {
        for item in &mut set.items {
            match item {
                Selection::Field(field) => {
                    // aliases and field names share the response keys, so they have to map
                    // the same way, `id: price` has to clash with `id` exactly like before
                    if let Some(alias) = &mut field.alias {
                        *alias = self.field_name(alias);
                    }
                    field.name = self.field_name(field.name);
                    self.arguments(&mut field.arguments);
                    self.directives(&mut field.directives);
                    self.selection_set(&mut field.selection_set);
                }
                Selection::FragmentSpread(spread) => {
                    spread.fragment_name = self.name(Kind::Fragment, spread.fragment_name);
                    self.directives(&mut spread.directives);
                }
                Selection::InlineFragment(fragment) => {
                    if let Some(TypeCondition::On(name)) = &mut fragment.type_condition {
                        *name = self.type_name(name);
                    }
                    self.directives(&mut fragment.directives);
                    self.selection_set(&mut fragment.selection_set);
                }
            }
        }
    }

    fn directives(&self, directives: &mut [Directive<'a, &'a str>]) {
        for directive in directives {
            if is_kept_directive(directive.name) {
                self.spec_directive(directive);
            } else {
                directive.name = self.name(Kind::Directive, directive.name);
                self.arguments(&mut directive.arguments);
            }
        }
    }

    // spec directives keep their name and argument names, the values are what carries user data
    fn spec_directive(&self, directive: &mut Directive<'a, &'a str>) {
        let name = directive.name;
        let spec_link = name == "link"
            && directive.arguments.iter().any(|(arg, value)| {
                *arg == "url" && matches!(value, Value::String(url) if url.starts_with(SPEC_URL))
            });
        for (arg, value) in &mut directive.arguments {
            match (name, *arg, value) {
                ("link", _, _) if spec_link => {}
                // a custom spec, like the ones `@composeDirective` brings in. It has to stay a
                // `.../name/vX.Y` URL, and its imports have to match the renamed directives
                ("link", "url", Value::String(s)) => {
                    let version = s.rsplit('/').next().filter(|v| v.starts_with('v'));
                    *s = format!(
                        "https://{}.invalid/{}/{}",
                        self.name(Kind::String, s).replace('_', "-"),
                        self.name(Kind::Directive, s),
                        version.unwrap_or("v1.0")
                    );
                }
                ("link", "import", Value::List(items)) => {
                    for item in items {
                        match item {
                            Value::String(s) => *s = self.import_name(s),
                            // `{ name: "@a", as: "@b" }`, the keys are part of the link spec
                            Value::Object(fields) => {
                                for (_, value) in fields {
                                    if let Value::String(s) = value {
                                        *s = self.import_name(s);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                ("link", "for", _) => {}
                ("core", _, _) => {}
                ("cost", "weight", _) => {}
                ("listSize", "assumedSize" | "requireOneSlicingArgument", _) => {}
                (
                    "join__type" | "join__field",
                    "key" | "requires" | "provides",
                    Value::String(s),
                ) => {
                    let fields = self.field_set(s);
                    rewrite(s, fields);
                }
                ("join__field", "type", Value::String(s))
                | ("join__implements", "interface", Value::String(s))
                | ("join__unionMember", "member", Value::String(s)) => self.type_string(s),
                ("join__graph", "name", Value::String(s))
                | ("join__field", "override", Value::String(s)) => {
                    let name = self.name(Kind::Subgraph, s);
                    rewrite(s, name);
                }
                // `.invalid` never resolves, so nothing can reach the real subgraph by accident
                ("join__graph", "url", Value::String(s)) => {
                    *s = format!(
                        "https://{}.invalid/graphql",
                        self.name(Kind::String, s).replace('_', "-")
                    )
                }
                // percentages drive progressive override, the planner has to see them
                ("join__field", "overrideLabel", Value::String(s)) if s.starts_with("percent(") => {
                }
                ("join__field", "contextArguments", Value::List(items)) => {
                    for item in items {
                        let Value::Object(fields) = item else {
                            self.value(item);
                            continue;
                        };
                        for (key, value) in fields {
                            match (*key, value) {
                                ("context", Value::String(s)) => {
                                    let name = self.name(Kind::Context, s);
                                    rewrite(s, name);
                                }
                                ("name", Value::String(s)) => {
                                    let name = self.name(Kind::Argument, s);
                                    rewrite(s, name);
                                }
                                ("type", Value::String(s)) => self.type_string(s),
                                ("selection", Value::String(s)) => {
                                    let fields = self.field_set(s);
                                    rewrite(s, fields);
                                }
                                (_, value) => self.value(value),
                            }
                        }
                    }
                }
                ("context", "name", Value::String(s)) => {
                    let name = self.name(Kind::Context, s);
                    rewrite(s, name);
                }
                ("join__directive", "name", Value::String(s)) if !is_kept_directive(s) => {
                    let name = self.name(Kind::Directive, s);
                    rewrite(s, name);
                }
                ("join__directive", "name", _) => {}
                ("join__directive", "args", Value::Object(fields)) => self.arguments(fields),
                // `input.first` is an argument followed by input fields
                ("listSize", "slicingArguments", Value::List(items)) => {
                    for item in items {
                        if let Value::String(path) = item {
                            *path = path
                                .split('.')
                                .enumerate()
                                .map(|(i, step)| {
                                    let kind = if i == 0 {
                                        Kind::Argument
                                    } else {
                                        Kind::InputField
                                    };
                                    self.name(kind, step)
                                })
                                .collect::<Vec<_>>()
                                .join(".");
                        }
                    }
                }
                ("listSize", "sizedFields", Value::List(items)) => {
                    for item in items {
                        if let Value::String(s) = item {
                            let fields = self.field_set(s);
                            rewrite(s, fields);
                        }
                    }
                }
                (_, _, value) => self.value(value),
            }
        }
    }

    fn fields(&self, fields: &mut [Field<'a, &'a str>]) {
        for field in fields {
            field.description = None;
            field.name = self.field_name(field.name);
            self.input_values(&mut field.arguments, Kind::Argument);
            self.type_ref(&mut field.field_type);
            self.directives(&mut field.directives);
        }
    }

    fn input_values(&self, values: &mut [InputValue<'a, &'a str>], kind: Kind) {
        for value in values {
            value.description = None;
            value.name = self.name(kind, value.name);
            self.type_ref(&mut value.value_type);
            if let Some(default) = &mut value.default_value {
                self.value(default);
            }
            self.directives(&mut value.directives);
        }
    }

    fn rename_all(&self, names: &mut [&'a str]) {
        for name in names {
            *name = self.type_name(name);
        }
    }

    // one definition at a time, straight into the output. Only the definition being worked on
    // is ever parsed, the rest of the supergraph is either still text or already written out
    fn supergraph(&self, sdl: &'a str, output: &mut impl std::io::Write) -> Result<(), String> {
        let style = Style::default();
        // reused for every definition, it ends up as big as the biggest one
        let mut buf = String::new();
        let mut empty = true;
        for definition in parse_schema_definitions::<&'a str>(sdl) {
            let mut definition = definition.map_err(|err| err.to_string())?;
            match &mut definition {
                Definition::SchemaDefinition(schema) => {
                    self.directives(&mut schema.directives);
                    for root in [
                        &mut schema.query,
                        &mut schema.mutation,
                        &mut schema.subscription,
                    ]
                    .into_iter()
                    .flatten()
                    {
                        *root = self.type_name(root);
                    }
                }
                Definition::DirectiveDefinition(directive) => {
                    directive.description = None;
                    if !is_kept_directive(directive.name) {
                        directive.name = self.name(Kind::Directive, directive.name);
                        self.input_values(&mut directive.arguments, Kind::Argument);
                    }
                }
                Definition::TypeDefinition(definition) => self.type_definition(definition),
                Definition::TypeExtension(_) => {
                    return Err("a supergraph can't have type extensions".into())
                }
            }
            // `format_into` only puts a blank line between definitions when the buffer isn't empty,
            // so keep a newline in it to get the same output as formatting the whole document
            buf.clear();
            if !empty {
                buf.push('\n');
            }
            definition.format_into(&style, &mut buf);
            let printed = if empty { &buf[..] } else { &buf[1..] };
            output
                .write_all(printed.as_bytes())
                .map_err(|err| err.to_string())?;
            empty = false;
        }
        if empty {
            return Err("the supergraph has no definitions".into());
        }
        Ok(())
    }

    fn type_definition(&self, definition: &mut TypeDefinition<'a, &'a str>) {
        let name = match definition {
            TypeDefinition::Scalar(t) => t.name,
            TypeDefinition::Object(t) => t.name,
            TypeDefinition::Interface(t) => t.name,
            TypeDefinition::Union(t) => t.name,
            TypeDefinition::Enum(t) => t.name,
            TypeDefinition::InputObject(t) => t.name,
        };
        // spec types stay as they are, except the subgraph names in `join__Graph`
        if is_spec(name) && name != "join__Graph" {
            return;
        }
        match definition {
            TypeDefinition::Scalar(scalar) => {
                scalar.description = None;
                scalar.name = self.type_name(scalar.name);
                self.directives(&mut scalar.directives);
            }
            TypeDefinition::Object(object) => {
                object.description = None;
                object.name = self.type_name(object.name);
                self.rename_all(&mut object.implements_interfaces);
                self.directives(&mut object.directives);
                self.fields(&mut object.fields);
            }
            TypeDefinition::Interface(interface) => {
                interface.description = None;
                interface.name = self.type_name(interface.name);
                self.rename_all(&mut interface.implements_interfaces);
                self.directives(&mut interface.directives);
                self.fields(&mut interface.fields);
            }
            TypeDefinition::Union(union) => {
                union.description = None;
                union.name = self.type_name(union.name);
                self.rename_all(&mut union.types);
                self.directives(&mut union.directives);
            }
            TypeDefinition::Enum(enum_type) => {
                enum_type.description = None;
                enum_type.name = self.type_name(enum_type.name);
                self.directives(&mut enum_type.directives);
                for value in &mut enum_type.values {
                    value.description = None;
                    value.name = self.name(Kind::EnumValue, value.name);
                    self.directives(&mut value.directives);
                }
            }
            TypeDefinition::InputObject(input) => {
                input.description = None;
                input.name = self.type_name(input.name);
                self.directives(&mut input.directives);
                self.input_values(&mut input.fields, Kind::InputField);
            }
        }
    }

    fn operation_document(&self, source: &'a str) -> Result<String, String> {
        let mut document = parse_query::<&'a str>(source).map_err(|err| err.to_string())?;
        for definition in &mut document.definitions {
            match definition {
                OperationDocDefinition::Operation(OperationDefinition::SelectionSet(set)) => {
                    self.selection_set(set)
                }
                OperationDocDefinition::Operation(OperationDefinition::Query(op)) => self
                    .operation(
                        &mut op.name,
                        &mut op.variable_definitions,
                        &mut op.directives,
                        &mut op.selection_set,
                    ),
                OperationDocDefinition::Operation(OperationDefinition::Mutation(op)) => self
                    .operation(
                        &mut op.name,
                        &mut op.variable_definitions,
                        &mut op.directives,
                        &mut op.selection_set,
                    ),
                OperationDocDefinition::Operation(OperationDefinition::Subscription(op)) => self
                    .operation(
                        &mut op.name,
                        &mut op.variable_definitions,
                        &mut op.directives,
                        &mut op.selection_set,
                    ),
                OperationDocDefinition::Fragment(fragment) => {
                    fragment.name = self.name(Kind::Fragment, fragment.name);
                    let TypeCondition::On(name) = &mut fragment.type_condition;
                    *name = self.type_name(name);
                    self.directives(&mut fragment.directives);
                    self.selection_set(&mut fragment.selection_set);
                }
            }
        }
        Ok(document.format(&Style::default()))
    }

    fn operation(
        &self,
        name: &mut Option<&'a str>,
        variables: &mut [VariableDefinition<'a, &'a str>],
        directives: &mut [Directive<'a, &'a str>],
        selection_set: &mut SelectionSet<'a, &'a str>,
    ) {
        if let Some(name) = name {
            *name = self.name(Kind::Operation, name);
        }
        for variable in variables {
            variable.name = self.name(Kind::Variable, variable.name);
            self.type_ref(&mut variable.var_type);
            if let Some(default) = &mut variable.default_value {
                self.value(default);
            }
        }
        self.directives(directives);
        self.selection_set(selection_set);
    }
}

// string values are owned by the parser, overwriting keeps their buffer
fn rewrite(string: &mut String, with: &str) {
    string.clear();
    string.push_str(with);
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))
}

/// `hive_router anonymize ...`, `args` are whatever comes after `anonymize`.
pub fn run_cli(args: &[String]) -> Result<(), String> {
    let mut key_file = None;
    let mut positional = vec![];
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "--key-file" {
            key_file = Some(args.next().ok_or(USAGE)?);
        } else {
            positional.push(arg.as_str());
        }
    }

    // never from argv, it'd show up in `ps` and shell history
    let anonymizer = || -> Result<Anonymizer, String> {
        let hex = match key_file {
            Some(path) => read(Path::new(path))?,
            None => std::env::var(KEY_ENV)
                .map_err(|_| format!("pass --key-file or set {KEY_ENV}\n\n{USAGE}"))?,
        };
        Anonymizer::from_hex(&hex)
    };

    match positional.as_slice() {
        ["keygen"] => println!("{}", Anonymizer::generate_key()?),
        ["supergraph", input] => {
            let sdl = read(Path::new(input))?;
            anonymizer()?.anonymize_supergraph_to(&sdl, &mut std::io::stdout().lock())?
        }
        ["operations", input] => {
            print!(
                "{}",
                anonymizer()?.anonymize_operation(&read(Path::new(input))?)?
            )
        }
        ["operations", input, output] => {
            let anonymizer = anonymizer()?;
            std::fs::create_dir_all(output).map_err(|err| format!("{output}: {err}"))?;
            let entries = std::fs::read_dir(input).map_err(|err| format!("{input}: {err}"))?;
            for entry in entries {
                let path = entry.map_err(|err| err.to_string())?.path();
                if !path.is_file() {
                    continue;
                }
                let anonymized = anonymizer
                    .anonymize_operation(&read(&path)?)
                    .map_err(|err| format!("{}: {err}", path.display()))?;
                // file names leak too, think `getUserSsn.graphql`
                let file_name = path.file_name().unwrap_or_default().to_string_lossy();
                let target = Path::new(output).join(format!(
                    "{}.graphql",
                    anonymizer.name(Kind::String, &file_name)
                ));
                std::fs::write(&target, anonymized)
                    .map_err(|err| format!("{}: {err}", target.display()))?;
                eprintln!("{} -> {}", path.display(), target.display());
            }
        }
        _ => return Err(USAGE.into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::PathBuf;

    use graphql_tools::parser::schema::{Definition, TypeDefinition};
    use graphql_tools::parser::{parse_query, parse_schema};

    use super::{is_kept_directive, is_spec, Anonymizer, Kind, KEPT_TYPES};
    use crate::query_planner::graph::Graph;
    use crate::query_planner::state::supergraph_state::SupergraphState;
    use crate::query_planner::tests::testkit::build_query_plan_from_sdl;
    use crate::query_planner::utils::parsing::{parse_operation, safe_parse_schema};

    fn anonymizer(byte: u8) -> Anonymizer {
        Anonymizer::new([byte; 32])
    }

    fn manifest_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn tokens(text: &str) -> HashSet<String> {
        text.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .filter(|token| !token.is_empty())
            .map(String::from)
            .collect()
    }

    fn supergraph_fixtures() -> Vec<PathBuf> {
        let mut found = vec![];
        let mut dirs = vec![manifest_dir().join("fixture")];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                // bfg is 30MB, way too slow for a debug build
                } else if path.metadata().unwrap().len() < 1_000_000
                    && std::fs::read_to_string(&path).is_ok_and(|sdl| sdl.contains("join__Graph"))
                {
                    found.push(path);
                }
            }
        }
        found.sort();
        found
    }

    // (supergraph, operation) pairs, pulled out of the planner tests so there are plenty
    fn planner_cases() -> Vec<(String, String)> {
        const OPERATION: &str = "parse_operation(";
        const PLAN: &str = "build_query_plan_with_defaults(";
        let mut cases = vec![];
        let dir = manifest_dir().join("src/query_planner/tests");
        for entry in std::fs::read_dir(dir).unwrap() {
            let Ok(source) = std::fs::read_to_string(entry.unwrap().path()) else {
                continue;
            };
            let mut rest = source.as_str();
            while let Some(start) = rest.find(OPERATION) {
                rest = &rest[start + OPERATION.len()..];
                let Some(query) = rest.trim_start().strip_prefix("r#\"") else {
                    continue;
                };
                let Some(end) = query.find("\"#") else {
                    break;
                };
                let after = &query[end..];
                // the plan call has to follow right away, otherwise it's not our pattern
                let Some(plan) = after.find(PLAN).filter(|plan| *plan < 80) else {
                    continue;
                };
                let path = after[plan + PLAN.len()..].trim_start();
                let Some(path) = path.strip_prefix('"').and_then(|p| p.split('"').next()) else {
                    continue;
                };
                cases.push((path.to_string(), query[..end].to_string()));
            }
        }
        // the bigger fixtures (heavy-query, abstract-many-subgraphs, bfg) take ages in a debug build
        for dir in ["", "products-list/"] {
            cases.push((
                format!("fixture/{dir}supergraph.graphql"),
                std::fs::read_to_string(
                    manifest_dir().join(format!("fixture/{dir}operation.graphql")),
                )
                .unwrap(),
            ));
        }
        cases
    }

    // strings differ by design, everything else (kinds, nesting, counts) has to match
    fn shape(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::String(_) => serde_json::Value::Null,
            serde_json::Value::Array(items) => items.iter().map(shape).collect(),
            serde_json::Value::Object(fields) => fields
                .iter()
                .map(|(key, value)| (key.clone(), shape(value)))
                .collect(),
            other => other.clone(),
        }
    }

    #[test]
    fn plans_keep_their_shape() {
        let cases = planner_cases();
        assert!(
            cases.len() > 100,
            "found only {} planner cases",
            cases.len()
        );
        let anonymizer = anonymizer(7);
        let mut planned = 0;
        for (supergraph, operation) in cases {
            let sdl = std::fs::read_to_string(manifest_dir().join(&supergraph)).unwrap();
            let Ok(expected) = build_query_plan_from_sdl(
                &sdl,
                parse_operation(&operation),
                Default::default(),
                Default::default(),
            ) else {
                continue;
            };
            let anonymous_operation = anonymizer.anonymize_operation(&operation).unwrap();
            let actual = build_query_plan_from_sdl(
                &anonymizer.anonymize_supergraph(&sdl).unwrap(),
                parse_operation(&anonymous_operation),
                Default::default(),
                Default::default(),
            )
            .unwrap_or_else(|err| panic!("{supergraph}\n{operation}\n{err}"));
            assert_eq!(
                shape(&serde_json::to_value(&expected).unwrap()),
                shape(&serde_json::to_value(&actual).unwrap()),
                "{supergraph}\n{operation}\n{expected}\n{actual}"
            );

            let allowed = [
                "query",
                "mutation",
                "subscription",
                "fragment",
                "on",
                "true",
                "false",
                "null",
                "__typename",
                "skip",
                "include",
                "if",
                "defer",
                "label",
            ];
            let leaked: Vec<_> = tokens(&anonymous_operation)
                .intersection(&tokens(&operation))
                .filter(|token| {
                    !allowed.contains(&token.as_str()) && !KEPT_TYPES.contains(&token.as_str())
                })
                .cloned()
                .collect();
            assert!(leaked.is_empty(), "{leaked:?} leaked from\n{operation}");
            planned += 1;
        }
        assert!(planned > 100, "planned only {planned} cases");
    }

    #[test]
    fn every_fixture_builds_the_same_graph() {
        let anonymizer = anonymizer(7);
        for path in supergraph_fixtures() {
            let sdl = std::fs::read_to_string(&path).unwrap();
            let Ok(schema) = safe_parse_schema(&sdl) else {
                continue;
            };
            let state = SupergraphState::new(&schema);
            let Ok(expected) = Graph::graph_from_supergraph_state(&state) else {
                continue;
            };
            let anonymous = anonymizer
                .anonymize_supergraph(&sdl)
                .unwrap_or_else(|err| panic!("{}: {err}", path.display()));
            let anonymous_state = SupergraphState::new(&safe_parse_schema(&anonymous).unwrap());
            let actual = Graph::graph_from_supergraph_state(&anonymous_state)
                .unwrap_or_else(|err| panic!("{}: {err}", path.display()));
            assert_eq!(
                expected.to_string().lines().count(),
                actual.to_string().lines().count(),
                "{}",
                path.display()
            );
        }
    }

    #[test]
    fn no_user_names_survive() {
        let anonymizer = anonymizer(7);
        for path in supergraph_fixtures() {
            let sdl = std::fs::read_to_string(&path).unwrap();
            let Ok(document) = parse_schema::<&str>(&sdl) else {
                continue;
            };
            // words the spec definitions use on their own, like `name` in `@join__graph(name:)`
            let mut spec_words = HashSet::new();
            let mut user_names = HashSet::new();
            for definition in &document.definitions {
                match definition {
                    Definition::DirectiveDefinition(d) if is_kept_directive(d.name) => {
                        spec_words.extend(tokens(&definition.to_string()));
                    }
                    Definition::TypeDefinition(TypeDefinition::Enum(e))
                        if e.name == "join__Graph" =>
                    {
                        for value in &e.values {
                            user_names.insert(value.name.to_string());
                            for directive in &value.directives {
                                for (_, value) in &directive.arguments {
                                    user_names.extend(tokens(&value.to_string()));
                                }
                            }
                        }
                    }
                    Definition::TypeDefinition(t) if is_spec(type_name(t)) => {
                        spec_words.extend(tokens(&definition.to_string()));
                    }
                    Definition::TypeDefinition(t) => {
                        user_names.insert(type_name(t).to_string());
                        match t {
                            TypeDefinition::Object(o) => o.fields.iter().for_each(|f| {
                                user_names.insert(f.name.to_string());
                                user_names.extend(f.arguments.iter().map(|a| a.name.to_string()));
                            }),
                            TypeDefinition::Interface(i) => i.fields.iter().for_each(|f| {
                                user_names.insert(f.name.to_string());
                                user_names.extend(f.arguments.iter().map(|a| a.name.to_string()));
                            }),
                            TypeDefinition::Enum(e) => {
                                user_names.extend(e.values.iter().map(|v| v.name.to_string()))
                            }
                            TypeDefinition::InputObject(i) => {
                                user_names.extend(i.fields.iter().map(|f| f.name.to_string()))
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            let anonymous = anonymizer.anonymize_supergraph(&sdl).unwrap();
            let output = tokens(&anonymous);
            let mut leaked: Vec<_> = user_names
                .iter()
                .filter(|name| {
                    // `https` and `graphql` come from the `https://....invalid/graphql` we write,
                    // the rest are SDL keywords
                    // numbers like the `0` in `127.0.0.1` aren't names
                    !name.starts_with(|c: char| c.is_ascii_digit())
                        && output.contains(*name)
                        && !spec_words.contains(*name)
                        && !KEPT_TYPES.contains(&name.as_str())
                        && ![
                            "graphql",
                            "query",
                            "mutation",
                            "subscription",
                            "https",
                            "input",
                            "type",
                            "enum",
                            "scalar",
                            "union",
                            "interface",
                            "schema",
                            "directive",
                            "on",
                            "implements",
                            "repeatable",
                        ]
                        .contains(&name.as_str())
                })
                .collect();
            leaked.sort();
            assert!(leaked.is_empty(), "{}: {leaked:?} leaked", path.display());
        }
    }

    fn type_name<'a>(definition: &TypeDefinition<'a, &'a str>) -> &'a str {
        match definition {
            TypeDefinition::Scalar(t) => t.name,
            TypeDefinition::Object(t) => t.name,
            TypeDefinition::Interface(t) => t.name,
            TypeDefinition::Union(t) => t.name,
            TypeDefinition::Enum(t) => t.name,
            TypeDefinition::InputObject(t) => t.name,
        }
    }

    // what `@composeDirective` leaves in a supergraph
    #[test]
    fn custom_link_imports_follow_the_directives() {
        let sdl = r#"
            schema
              @link(url: "https://specs.apollo.dev/link/v1.0")
              @link(url: "https://specs.apollo.dev/join/v0.5", for: EXECUTION)
              @link(url: "https://myspecs.dev/custom/v1.2", import: ["@custom", { name: "@other", as: "@renamed" }])
            {
              query: Query
            }

            directive @custom(label: String) on FIELD_DEFINITION | OBJECT
            directive @renamed on FIELD_DEFINITION
            directive @join__graph(name: String!, url: String!) on ENUM_VALUE
            directive @join__type(graph: join__Graph!, key: join__FieldSet, extension: Boolean! = false, resolvable: Boolean! = true, isInterfaceObject: Boolean! = false) repeatable on OBJECT | INTERFACE | UNION | ENUM | INPUT_OBJECT | SCALAR
            directive @link(url: String, as: String, for: link__Purpose, import: [link__Import]) repeatable on SCHEMA

            scalar join__FieldSet
            scalar link__Import
            enum join__Graph { A @join__graph(name: "a", url: "http://a") }
            enum link__Purpose { SECURITY EXECUTION }

            type Query @join__type(graph: A) @custom(label: "secret") {
              product: ID @renamed
            }
        "#;
        let out = anonymizer(1).anonymize_supergraph(sdl).unwrap();
        let custom = anonymizer(1).name(Kind::Directive, "custom");
        let renamed = anonymizer(1).name(Kind::Directive, "renamed");
        assert!(out.contains(&format!("directive @{custom}(")), "{out}");
        assert!(out.contains(&format!("import: [\"@{custom}\", {{name: \"@")), "{out}");
        assert!(out.contains(&format!("as: \"@{renamed}\"")), "{out}");
        assert!(out.contains(".invalid/d_") && out.contains("/v1.2\""), "{out}");
        for secret in ["myspecs", "custom", "other", "renamed", "secret"] {
            assert!(!out.contains(secret), "{secret} survived:\n{out}");
        }
    }

    #[test]
    fn same_key_same_output() {
        let sdl =
            std::fs::read_to_string(manifest_dir().join("fixture/supergraph.graphql")).unwrap();
        let operation = "query Q($id: ID!) { user(id: $id) { id name } }";
        assert_eq!(
            anonymizer(1).anonymize_supergraph(&sdl).unwrap(),
            anonymizer(1).anonymize_supergraph(&sdl).unwrap()
        );
        assert_eq!(
            anonymizer(1).anonymize_operation(operation).unwrap(),
            anonymizer(1).anonymize_operation(operation).unwrap()
        );
        let one = anonymizer(1).anonymize_operation(operation).unwrap();
        let two = anonymizer(2).anonymize_operation(operation).unwrap();
        assert!(tokens(&one)
            .intersection(&tokens(&two))
            .all(|token| ["query", "ID"].contains(&token.as_str())));
        assert!(parse_query::<String>(&one).is_ok());

        let key = Anonymizer::generate_key().unwrap();
        assert_eq!(key.len(), 64);
        assert!(Anonymizer::from_hex(&key).is_ok());
        assert!(Anonymizer::from_hex("abc").is_err());
    }
}
