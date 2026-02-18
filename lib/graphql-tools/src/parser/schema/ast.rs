use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

use thiserror::Error;

pub use crate::parser::common::{Directive, Text, Type, Value};
use crate::parser::position::Pos;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Document<'a, T: Text<'a>> {
    pub definitions: Vec<Definition<'a, T>>,
}

impl<'a> Document<'a, String> {
    pub fn into_static(self) -> Document<'static, String> {
        // To support both reference and owned values in the AST,
        // all string data is represented with the ::common::Str<'a, T: Text<'a>>
        // wrapper type.
        // This type must carry the liftetime of the schema string,
        // and is stored in a PhantomData value on the Str type.
        // When using owned String types, the actual lifetime of
        // the Ast nodes is 'static, since no references are kept,
        // but the nodes will still carry the input lifetime.
        // To continue working with Document<String> in a owned fasion
        // the lifetime needs to be transmuted to 'static.
        //
        // This is safe because no references are present.
        // Just the PhantomData lifetime reference is transmuted away.
        unsafe { std::mem::transmute::<_, Document<'static, String>>(self) }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Definition<'a, T: Text<'a>> {
    SchemaDefinition(SchemaDefinition<'a, T>),
    TypeDefinition(TypeDefinition<'a, T>),
    TypeExtension(TypeExtension<'a, T>),
    DirectiveDefinition(DirectiveDefinition<'a, T>),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SchemaDefinition<'a, T: Text<'a>> {
    pub position: Pos,
    pub directives: Vec<Directive<'a, T>>,
    pub query: Option<T::Value>,
    pub mutation: Option<T::Value>,
    pub subscription: Option<T::Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeDefinition<'a, T: Text<'a>> {
    Scalar(ScalarType<'a, T>),
    Object(ObjectType<'a, T>),
    Interface(InterfaceType<'a, T>),
    Union(UnionType<'a, T>),
    Enum(EnumType<'a, T>),
    InputObject(InputObjectType<'a, T>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeExtension<'a, T: Text<'a>> {
    Scalar(ScalarTypeExtension<'a, T>),
    Object(ObjectTypeExtension<'a, T>),
    Interface(InterfaceTypeExtension<'a, T>),
    Union(UnionTypeExtension<'a, T>),
    Enum(EnumTypeExtension<'a, T>),
    InputObject(InputObjectTypeExtension<'a, T>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScalarType<'a, T: Text<'a>> {
    pub position: Pos,
    pub description: Option<String>,
    pub name: T::Value,
    pub directives: Vec<Directive<'a, T>>,
}

impl<'a, T> ScalarType<'a, T>
where
    T: Text<'a>,
{
    pub fn new(name: T::Value) -> Self {
        Self {
            position: Pos::default(),
            description: None,
            name,
            directives: vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScalarTypeExtension<'a, T: Text<'a>> {
    pub position: Pos,
    pub name: T::Value,
    pub directives: Vec<Directive<'a, T>>,
}

impl<'a, T> ScalarTypeExtension<'a, T>
where
    T: Text<'a>,
{
    pub fn new(name: T::Value) -> Self {
        Self {
            position: Pos::default(),
            name,
            directives: vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObjectType<'a, T: Text<'a>> {
    pub position: Pos,
    pub description: Option<String>,
    pub name: T::Value,
    pub implements_interfaces: Vec<T::Value>,
    pub directives: Vec<Directive<'a, T>>,
    pub fields: Vec<Field<'a, T>>,
}

impl<'a, T> ObjectType<'a, T>
where
    T: Text<'a>,
{
    pub fn new(name: T::Value) -> Self {
        Self {
            position: Pos::default(),
            description: None,
            name,
            implements_interfaces: vec![],
            directives: vec![],
            fields: vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObjectTypeExtension<'a, T: Text<'a>> {
    pub position: Pos,
    pub name: T::Value,
    pub implements_interfaces: Vec<T::Value>,
    pub directives: Vec<Directive<'a, T>>,
    pub fields: Vec<Field<'a, T>>,
}

impl<'a, T> ObjectTypeExtension<'a, T>
where
    T: Text<'a>,
{
    pub fn new(name: T::Value) -> Self {
        Self {
            position: Pos::default(),
            name,
            implements_interfaces: vec![],
            directives: vec![],
            fields: vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field<'a, T: Text<'a>> {
    pub position: Pos,
    pub description: Option<String>,
    pub name: T::Value,
    pub arguments: Vec<InputValue<'a, T>>,
    pub field_type: Type<'a, T>,
    pub directives: Vec<Directive<'a, T>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InputValue<'a, T: Text<'a>> {
    pub position: Pos,
    pub description: Option<String>,
    pub name: T::Value,
    pub value_type: Type<'a, T>,
    pub default_value: Option<Value<'a, T>>,
    pub directives: Vec<Directive<'a, T>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceType<'a, T: Text<'a>> {
    pub position: Pos,
    pub description: Option<String>,
    pub name: T::Value,
    pub implements_interfaces: Vec<T::Value>,
    pub directives: Vec<Directive<'a, T>>,
    pub fields: Vec<Field<'a, T>>,
}

impl<'a, T> InterfaceType<'a, T>
where
    T: Text<'a>,
{
    pub fn new(name: T::Value) -> Self {
        Self {
            position: Pos::default(),
            description: None,
            name,
            implements_interfaces: vec![],
            directives: vec![],
            fields: vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceTypeExtension<'a, T: Text<'a>> {
    pub position: Pos,
    pub name: T::Value,
    pub implements_interfaces: Vec<T::Value>,
    pub directives: Vec<Directive<'a, T>>,
    pub fields: Vec<Field<'a, T>>,
}

impl<'a, T> InterfaceTypeExtension<'a, T>
where
    T: Text<'a>,
{
    pub fn new(name: T::Value) -> Self {
        Self {
            position: Pos::default(),
            name,
            implements_interfaces: vec![],
            directives: vec![],
            fields: vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct UnionType<'a, T: Text<'a>> {
    pub position: Pos,
    pub description: Option<String>,
    pub name: T::Value,
    pub directives: Vec<Directive<'a, T>>,
    pub types: Vec<T::Value>,
}

impl<'a, T> UnionType<'a, T>
where
    T: Text<'a>,
{
    pub fn new(name: T::Value) -> Self {
        Self {
            position: Pos::default(),
            description: None,
            name,
            directives: vec![],
            types: vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct UnionTypeExtension<'a, T: Text<'a>> {
    pub position: Pos,
    pub name: T::Value,
    pub directives: Vec<Directive<'a, T>>,
    pub types: Vec<T::Value>,
}

impl<'a, T> UnionTypeExtension<'a, T>
where
    T: Text<'a>,
{
    pub fn new(name: T::Value) -> Self {
        Self {
            position: Pos::default(),
            name,
            directives: vec![],
            types: vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumType<'a, T: Text<'a>> {
    pub position: Pos,
    pub description: Option<String>,
    pub name: T::Value,
    pub directives: Vec<Directive<'a, T>>,
    pub values: Vec<EnumValue<'a, T>>,
}

impl<'a, T> EnumType<'a, T>
where
    T: Text<'a>,
{
    pub fn new(name: T::Value) -> Self {
        Self {
            position: Pos::default(),
            description: None,
            name,
            directives: vec![],
            values: vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumValue<'a, T: Text<'a>> {
    pub position: Pos,
    pub description: Option<String>,
    pub name: T::Value,
    pub directives: Vec<Directive<'a, T>>,
}

impl<'a, T> EnumValue<'a, T>
where
    T: Text<'a>,
{
    pub fn new(name: T::Value) -> Self {
        Self {
            position: Pos::default(),
            description: None,
            name,
            directives: vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumTypeExtension<'a, T: Text<'a>> {
    pub position: Pos,
    pub name: T::Value,
    pub directives: Vec<Directive<'a, T>>,
    pub values: Vec<EnumValue<'a, T>>,
}

impl<'a, T> EnumTypeExtension<'a, T>
where
    T: Text<'a>,
{
    pub fn new(name: T::Value) -> Self {
        Self {
            position: Pos::default(),
            name,
            directives: vec![],
            values: vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct InputObjectType<'a, T: Text<'a>> {
    pub position: Pos,
    pub description: Option<String>,
    pub name: T::Value,
    pub directives: Vec<Directive<'a, T>>,
    pub fields: Vec<InputValue<'a, T>>,
}

impl<'a, T> InputObjectType<'a, T>
where
    T: Text<'a>,
{
    pub fn new(name: T::Value) -> Self {
        Self {
            position: Pos::default(),
            description: None,
            name,
            directives: vec![],
            fields: vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct InputObjectTypeExtension<'a, T: Text<'a>> {
    pub position: Pos,
    pub name: T::Value,
    pub directives: Vec<Directive<'a, T>>,
    pub fields: Vec<InputValue<'a, T>>,
}

impl<'a, T> InputObjectTypeExtension<'a, T>
where
    T: Text<'a>,
{
    pub fn new(name: T::Value) -> Self {
        Self {
            position: Pos::default(),
            name,
            directives: vec![],
            fields: vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DirectiveLocation {
    // executable
    Query,
    Mutation,
    Subscription,
    Field,
    FragmentDefinition,
    FragmentSpread,
    InlineFragment,

    // type_system
    Schema,
    Scalar,
    Object,
    FieldDefinition,
    ArgumentDefinition,
    Interface,
    Union,
    Enum,
    EnumValue,
    InputObject,
    InputFieldDefinition,
    VariableDefinition,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DirectiveDefinition<'a, T: Text<'a>> {
    pub position: Pos,
    pub description: Option<String>,
    pub name: T::Value,
    pub arguments: Vec<InputValue<'a, T>>,
    pub repeatable: bool,
    pub locations: Vec<DirectiveLocation>,
}

impl<'a, T> DirectiveDefinition<'a, T>
where
    T: Text<'a>,
{
    pub fn new(name: T::Value) -> Self {
        Self {
            position: Pos::default(),
            description: None,
            name,
            arguments: vec![],
            repeatable: false,
            locations: vec![],
        }
    }
}

impl DirectiveLocation {
    /// Returns GraphQL syntax compatible name of the directive
    pub fn as_str(&self) -> &'static str {
        use self::DirectiveLocation::*;
        match *self {
            Query => "QUERY",
            Mutation => "MUTATION",
            Subscription => "SUBSCRIPTION",
            Field => "FIELD",
            FragmentDefinition => "FRAGMENT_DEFINITION",
            FragmentSpread => "FRAGMENT_SPREAD",
            InlineFragment => "INLINE_FRAGMENT",
            Schema => "SCHEMA",
            Scalar => "SCALAR",
            Object => "OBJECT",
            FieldDefinition => "FIELD_DEFINITION",
            ArgumentDefinition => "ARGUMENT_DEFINITION",
            Interface => "INTERFACE",
            Union => "UNION",
            Enum => "ENUM",
            EnumValue => "ENUM_VALUE",
            InputObject => "INPUT_OBJECT",
            InputFieldDefinition => "INPUT_FIELD_DEFINITION",
            VariableDefinition => "VARIABLE_DEFINITION",
        }
    }

    /// Returns `true` if this location is for queries (execution)
    pub fn is_query(&self) -> bool {
        use self::DirectiveLocation::*;
        match *self {
            Query | Mutation | Subscription | Field | FragmentDefinition | FragmentSpread
            | InlineFragment => true,

            Schema | Scalar | Object | FieldDefinition | ArgumentDefinition | Interface | Union
            | Enum | EnumValue | InputObject | InputFieldDefinition | VariableDefinition => false,
        }
    }

    /// Returns `true` if this location is for schema
    pub fn is_schema(&self) -> bool {
        !self.is_query()
    }
}

#[derive(Debug, Error)]
#[error("invalid directive location")]
pub struct InvalidDirectiveLocation;

impl FromStr for DirectiveLocation {
    type Err = InvalidDirectiveLocation;
    fn from_str(s: &str) -> Result<DirectiveLocation, InvalidDirectiveLocation> {
        use self::DirectiveLocation::*;
        let val = match s {
            "QUERY" => Query,
            "MUTATION" => Mutation,
            "SUBSCRIPTION" => Subscription,
            "FIELD" => Field,
            "FRAGMENT_DEFINITION" => FragmentDefinition,
            "FRAGMENT_SPREAD" => FragmentSpread,
            "INLINE_FRAGMENT" => InlineFragment,
            "SCHEMA" => Schema,
            "SCALAR" => Scalar,
            "OBJECT" => Object,
            "FIELD_DEFINITION" => FieldDefinition,
            "ARGUMENT_DEFINITION" => ArgumentDefinition,
            "INTERFACE" => Interface,
            "UNION" => Union,
            "ENUM" => Enum,
            "ENUM_VALUE" => EnumValue,
            "INPUT_OBJECT" => InputObject,
            "INPUT_FIELD_DEFINITION" => InputFieldDefinition,
            "VARIABLE_DEFINITION" => VariableDefinition,
            _ => return Err(InvalidDirectiveLocation),
        };

        Ok(val)
    }
}

impl<'a, T: Text<'a>> TypeDefinition<'a, T> {
    pub fn directives(&self) -> &[Directive<'a, T>] {
        match self {
            TypeDefinition::Scalar(scalar) => &scalar.directives,
            TypeDefinition::Object(object) => &object.directives,
            TypeDefinition::Interface(interface) => &interface.directives,
            TypeDefinition::Union(union) => &union.directives,
            TypeDefinition::Enum(enum_) => &enum_.directives,
            TypeDefinition::InputObject(input_object) => &input_object.directives,
        }
    }
}

#[inline]
fn digest_of<F>(f: F) -> u64
where
    F: FnOnce(&mut DefaultHasher),
{
    let mut hasher = DefaultHasher::new();
    f(&mut hasher);
    hasher.finish()
}

const UNORDERED_MULTISET_DOMAIN: &str = "UnorderedMultisetV1";

/// Hashes an unordered multiset of element digests without allocations.
///
/// Properties:
/// - order-independent (`{a,b,c}` equals `{c,b,a}`)
/// - multiplicity-sensitive (`{a,a,b}` differs from `{a,b}`)
/// - deterministic for this implementation
///
/// This is intentionally non-cryptographic and used for cache identity only.
///
/// Implementation note:
/// - keeps three commutative accumulators (`count`, `xor`, `sum`)
/// - avoids domain-specific constants for readability and maintenance
#[inline]
fn hash_unordered_iter<H, I>(iter: I, state: &mut H)
where
    H: Hasher,
    I: IntoIterator<Item = u64>,
{
    let mut count: u64 = 0;
    let mut xor_acc: u64 = 0;
    let mut sum: u64 = 0;

    for digest in iter {
        count = count.wrapping_add(1);
        xor_acc ^= digest;
        sum = sum.wrapping_add(digest);
    }

    UNORDERED_MULTISET_DOMAIN.hash(state);
    count.hash(state);
    xor_acc.hash(state);
    sum.hash(state);
}

#[inline]
fn hash_text_value<'a, T, H>(value: &T::Value, state: &mut H)
where
    T: Text<'a>,
    H: Hasher,
{
    value.as_ref().hash(state);
}

#[inline]
fn hash_type_value<'a, T, H>(value: &Type<'a, T>, state: &mut H)
where
    T: Text<'a>,
    H: Hasher,
{
    match value {
        Type::NamedType(name) => {
            "Type::NamedType".hash(state);
            hash_text_value::<T, H>(name, state);
        }
        Type::ListType(inner) => {
            "Type::ListType".hash(state);
            hash_type_value::<T, H>(inner, state);
        }
        Type::NonNullType(inner) => {
            "Type::NonNullType".hash(state);
            hash_type_value::<T, H>(inner, state);
        }
    }
}

#[inline]
fn hash_const_value<'a, T, H>(value: &Value<'a, T>, state: &mut H)
where
    T: Text<'a>,
    H: Hasher,
{
    match value {
        Value::Variable(name) => {
            "Value::Variable".hash(state);
            hash_text_value::<T, H>(name, state);
        }
        Value::Int(number) => {
            "Value::Int".hash(state);
            number.as_i64().hash(state);
        }
        Value::Float(value) => {
            "Value::Float".hash(state);
            value.to_bits().hash(state);
        }
        Value::String(value) => {
            "Value::String".hash(state);
            value.hash(state);
        }
        Value::Boolean(value) => {
            "Value::Boolean".hash(state);
            value.hash(state);
        }
        Value::Null => {
            "Value::Null".hash(state);
        }
        Value::Enum(value) => {
            "Value::Enum".hash(state);
            hash_text_value::<T, H>(value, state);
        }
        Value::List(values) => {
            "Value::List".hash(state);
            values.len().hash(state);
            for item in values {
                hash_const_value::<T, H>(item, state);
            }
        }
        Value::Object(values) => {
            "Value::Object".hash(state);
            hash_unordered_iter(
                values.iter().map(|(key, value)| {
                    digest_of(|hasher| {
                        hash_text_value::<T, _>(key, hasher);
                        hash_const_value::<T, _>(value, hasher);
                    })
                }),
                state,
            );
        }
    }
}

#[inline]
fn hash_directive_value<'a, T, H>(directive: &Directive<'a, T>, state: &mut H)
where
    T: Text<'a>,
    H: Hasher,
{
    hash_text_value::<T, H>(&directive.name, state);
    hash_unordered_iter(
        directive.arguments.iter().map(|(name, value)| {
            digest_of(|hasher| {
                hash_text_value::<T, _>(name, hasher);
                hash_const_value::<T, _>(value, hasher);
            })
        }),
        state,
    );
}

impl<'a, T: Text<'a>> Hash for Document<'a, T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "Document".hash(state);
        hash_unordered_iter(
            self.definitions
                .iter()
                .map(|definition| digest_of(|hasher| definition.hash(hasher))),
            state,
        );
    }
}

impl<'a, T: Text<'a>> Hash for Definition<'a, T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::SchemaDefinition(value) => {
                "Definition::SchemaDefinition".hash(state);
                value.hash(state);
            }
            Self::TypeDefinition(value) => {
                "Definition::TypeDefinition".hash(state);
                value.hash(state);
            }
            Self::TypeExtension(value) => {
                "Definition::TypeExtension".hash(state);
                value.hash(state);
            }
            Self::DirectiveDefinition(value) => {
                "Definition::DirectiveDefinition".hash(state);
                value.hash(state);
            }
        }
    }
}

impl<'a, T: Text<'a>> Hash for SchemaDefinition<'a, T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "SchemaDefinition".hash(state);
        self.query.as_ref().map(AsRef::as_ref).hash(state);
        self.mutation.as_ref().map(AsRef::as_ref).hash(state);
        self.subscription.as_ref().map(AsRef::as_ref).hash(state);
        hash_unordered_iter(
            self.directives
                .iter()
                .map(|directive| digest_of(|hasher| hash_directive_value(directive, hasher))),
            state,
        );
    }
}

impl<'a, T: Text<'a>> Hash for TypeDefinition<'a, T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Scalar(value) => {
                "TypeDefinition::Scalar".hash(state);
                value.hash(state);
            }
            Self::Object(value) => {
                "TypeDefinition::Object".hash(state);
                value.hash(state);
            }
            Self::Interface(value) => {
                "TypeDefinition::Interface".hash(state);
                value.hash(state);
            }
            Self::Union(value) => {
                "TypeDefinition::Union".hash(state);
                value.hash(state);
            }
            Self::Enum(value) => {
                "TypeDefinition::Enum".hash(state);
                value.hash(state);
            }
            Self::InputObject(value) => {
                "TypeDefinition::InputObject".hash(state);
                value.hash(state);
            }
        }
    }
}

impl<'a, T: Text<'a>> Hash for TypeExtension<'a, T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Scalar(value) => {
                "TypeExtension::Scalar".hash(state);
                value.hash(state);
            }
            Self::Object(value) => {
                "TypeExtension::Object".hash(state);
                value.hash(state);
            }
            Self::Interface(value) => {
                "TypeExtension::Interface".hash(state);
                value.hash(state);
            }
            Self::Union(value) => {
                "TypeExtension::Union".hash(state);
                value.hash(state);
            }
            Self::Enum(value) => {
                "TypeExtension::Enum".hash(state);
                value.hash(state);
            }
            Self::InputObject(value) => {
                "TypeExtension::InputObject".hash(state);
                value.hash(state);
            }
        }
    }
}

impl<'a, T: Text<'a>> Hash for ScalarType<'a, T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "ScalarType".hash(state);
        self.description.hash(state);
        hash_text_value::<T, H>(&self.name, state);
        hash_unordered_iter(
            self.directives
                .iter()
                .map(|directive| digest_of(|hasher| hash_directive_value(directive, hasher))),
            state,
        );
    }
}

impl<'a, T: Text<'a>> Hash for ScalarTypeExtension<'a, T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "ScalarTypeExtension".hash(state);
        hash_text_value::<T, H>(&self.name, state);
        hash_unordered_iter(
            self.directives
                .iter()
                .map(|directive| digest_of(|hasher| hash_directive_value(directive, hasher))),
            state,
        );
    }
}

impl<'a, T: Text<'a>> Hash for ObjectType<'a, T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "ObjectType".hash(state);
        self.description.hash(state);
        hash_text_value::<T, H>(&self.name, state);
        hash_unordered_iter(
            self.implements_interfaces.iter().map(|interface_name| {
                digest_of(|hasher| hash_text_value::<T, _>(interface_name, hasher))
            }),
            state,
        );
        hash_unordered_iter(
            self.directives
                .iter()
                .map(|directive| digest_of(|hasher| hash_directive_value(directive, hasher))),
            state,
        );
        hash_unordered_iter(
            self.fields
                .iter()
                .map(|field| digest_of(|hasher| field.hash(hasher))),
            state,
        );
    }
}

impl<'a, T: Text<'a>> Hash for ObjectTypeExtension<'a, T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "ObjectTypeExtension".hash(state);
        hash_text_value::<T, H>(&self.name, state);
        hash_unordered_iter(
            self.implements_interfaces.iter().map(|interface_name| {
                digest_of(|hasher| hash_text_value::<T, _>(interface_name, hasher))
            }),
            state,
        );
        hash_unordered_iter(
            self.directives
                .iter()
                .map(|directive| digest_of(|hasher| hash_directive_value(directive, hasher))),
            state,
        );
        hash_unordered_iter(
            self.fields
                .iter()
                .map(|field| digest_of(|hasher| field.hash(hasher))),
            state,
        );
    }
}

impl<'a, T: Text<'a>> Hash for Field<'a, T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "Field".hash(state);
        self.description.hash(state);
        hash_text_value::<T, H>(&self.name, state);
        hash_unordered_iter(
            self.arguments
                .iter()
                .map(|argument| digest_of(|hasher| argument.hash(hasher))),
            state,
        );
        hash_type_value::<T, H>(&self.field_type, state);
        hash_unordered_iter(
            self.directives
                .iter()
                .map(|directive| digest_of(|hasher| hash_directive_value(directive, hasher))),
            state,
        );
    }
}

impl<'a, T: Text<'a>> Hash for InputValue<'a, T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "InputValue".hash(state);
        self.description.hash(state);
        hash_text_value::<T, H>(&self.name, state);
        hash_type_value::<T, H>(&self.value_type, state);
        self.default_value.is_some().hash(state);
        if let Some(default_value) = self.default_value.as_ref() {
            hash_const_value::<T, H>(default_value, state);
        }
        hash_unordered_iter(
            self.directives
                .iter()
                .map(|directive| digest_of(|hasher| hash_directive_value(directive, hasher))),
            state,
        );
    }
}

impl<'a, T: Text<'a>> Hash for InterfaceType<'a, T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "InterfaceType".hash(state);
        self.description.hash(state);
        hash_text_value::<T, H>(&self.name, state);
        hash_unordered_iter(
            self.implements_interfaces.iter().map(|interface_name| {
                digest_of(|hasher| hash_text_value::<T, _>(interface_name, hasher))
            }),
            state,
        );
        hash_unordered_iter(
            self.directives
                .iter()
                .map(|directive| digest_of(|hasher| hash_directive_value(directive, hasher))),
            state,
        );
        hash_unordered_iter(
            self.fields
                .iter()
                .map(|field| digest_of(|hasher| field.hash(hasher))),
            state,
        );
    }
}

impl<'a, T: Text<'a>> Hash for InterfaceTypeExtension<'a, T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "InterfaceTypeExtension".hash(state);
        hash_text_value::<T, H>(&self.name, state);
        hash_unordered_iter(
            self.implements_interfaces.iter().map(|interface_name| {
                digest_of(|hasher| hash_text_value::<T, _>(interface_name, hasher))
            }),
            state,
        );
        hash_unordered_iter(
            self.directives
                .iter()
                .map(|directive| digest_of(|hasher| hash_directive_value(directive, hasher))),
            state,
        );
        hash_unordered_iter(
            self.fields
                .iter()
                .map(|field| digest_of(|hasher| field.hash(hasher))),
            state,
        );
    }
}

impl<'a, T: Text<'a>> Hash for UnionType<'a, T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "UnionType".hash(state);
        self.description.hash(state);
        hash_text_value::<T, H>(&self.name, state);
        hash_unordered_iter(
            self.directives
                .iter()
                .map(|directive| digest_of(|hasher| hash_directive_value(directive, hasher))),
            state,
        );
        hash_unordered_iter(
            self.types
                .iter()
                .map(|type_name| digest_of(|hasher| hash_text_value::<T, _>(type_name, hasher))),
            state,
        );
    }
}

impl<'a, T: Text<'a>> Hash for UnionTypeExtension<'a, T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "UnionTypeExtension".hash(state);
        hash_text_value::<T, H>(&self.name, state);
        hash_unordered_iter(
            self.directives
                .iter()
                .map(|directive| digest_of(|hasher| hash_directive_value(directive, hasher))),
            state,
        );
        hash_unordered_iter(
            self.types
                .iter()
                .map(|type_name| digest_of(|hasher| hash_text_value::<T, _>(type_name, hasher))),
            state,
        );
    }
}

impl<'a, T: Text<'a>> Hash for EnumType<'a, T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "EnumType".hash(state);
        self.description.hash(state);
        hash_text_value::<T, H>(&self.name, state);
        hash_unordered_iter(
            self.directives
                .iter()
                .map(|directive| digest_of(|hasher| hash_directive_value(directive, hasher))),
            state,
        );
        hash_unordered_iter(
            self.values
                .iter()
                .map(|enum_value| digest_of(|hasher| enum_value.hash(hasher))),
            state,
        );
    }
}

impl<'a, T: Text<'a>> Hash for EnumValue<'a, T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "EnumValue".hash(state);
        self.description.hash(state);
        hash_text_value::<T, H>(&self.name, state);
        hash_unordered_iter(
            self.directives
                .iter()
                .map(|directive| digest_of(|hasher| hash_directive_value(directive, hasher))),
            state,
        );
    }
}

impl<'a, T: Text<'a>> Hash for EnumTypeExtension<'a, T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "EnumTypeExtension".hash(state);
        hash_text_value::<T, H>(&self.name, state);
        hash_unordered_iter(
            self.directives
                .iter()
                .map(|directive| digest_of(|hasher| hash_directive_value(directive, hasher))),
            state,
        );
        hash_unordered_iter(
            self.values
                .iter()
                .map(|enum_value| digest_of(|hasher| enum_value.hash(hasher))),
            state,
        );
    }
}

impl<'a, T: Text<'a>> Hash for InputObjectType<'a, T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "InputObjectType".hash(state);
        self.description.hash(state);
        hash_text_value::<T, H>(&self.name, state);
        hash_unordered_iter(
            self.directives
                .iter()
                .map(|directive| digest_of(|hasher| hash_directive_value(directive, hasher))),
            state,
        );
        hash_unordered_iter(
            self.fields
                .iter()
                .map(|field| digest_of(|hasher| field.hash(hasher))),
            state,
        );
    }
}

impl<'a, T: Text<'a>> Hash for InputObjectTypeExtension<'a, T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "InputObjectTypeExtension".hash(state);
        hash_text_value::<T, H>(&self.name, state);
        hash_unordered_iter(
            self.directives
                .iter()
                .map(|directive| digest_of(|hasher| hash_directive_value(directive, hasher))),
            state,
        );
        hash_unordered_iter(
            self.fields
                .iter()
                .map(|field| digest_of(|hasher| field.hash(hasher))),
            state,
        );
    }
}

impl<'a, T: Text<'a>> Hash for DirectiveDefinition<'a, T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "DirectiveDefinition".hash(state);
        self.description.hash(state);
        hash_text_value::<T, H>(&self.name, state);
        hash_unordered_iter(
            self.arguments
                .iter()
                .map(|argument| digest_of(|hasher| argument.hash(hasher))),
            state,
        );
        self.repeatable.hash(state);
        hash_unordered_iter(
            self.locations
                .iter()
                .map(|location| digest_of(|hasher| location.hash(hasher))),
            state,
        );
    }
}

#[cfg(test)]
mod tests {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    use super::*;
    use crate::parser::schema::parse_schema;

    fn hash_of<T: Hash>(value: &T) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    fn parse_doc(sdl: &str) -> Document<'static, String> {
        parse_schema::<String>(sdl)
            .expect("schema should parse")
            .to_owned()
            .into_static()
    }

    #[test]
    fn schema_hash_is_deterministic() {
        let doc = parse_doc(
            r#"
            directive @tag(a: Int, b: Int) on FIELD_DEFINITION | OBJECT

            type Query @tag(a: 1, b: 2) {
                user(id: ID!, role: String): String @tag(a: 1, b: 2)
                ping: String
            }
            "#,
        );

        let h1 = hash_of(&doc);
        let h2 = hash_of(&doc);
        let h3 = hash_of(&doc.clone());

        assert_eq!(h1, h2);
        assert_eq!(h2, h3);
    }

    #[test]
    fn schema_hash_is_order_independent() {
        let doc_a = parse_doc(
            r#"
            directive @tag(a: Int, b: Int) on FIELD_DEFINITION | OBJECT

            type User {
                id: ID!
                name: String
            }

            type Query @tag(a: 1, b: 2) {
                ping: String
                user(role: String, id: ID!): String @tag(a: 1, b: 2)
            }
            "#,
        );

        let doc_b = parse_doc(
            r#"
            type Query @tag(b: 2, a: 1) {
                user(id: ID!, role: String): String @tag(b: 2, a: 1)
                ping: String
            }

            directive @tag(b: Int, a: Int) on OBJECT | FIELD_DEFINITION

            type User {
                name: String
                id: ID!
            }
            "#,
        );

        assert_eq!(hash_of(&doc_a), hash_of(&doc_b));
    }

    #[test]
    fn schema_hash_is_multiplicity_sensitive() {
        let single: Document<'static, String> = Document {
            definitions: vec![Definition::SchemaDefinition(SchemaDefinition::default())],
        };
        let duplicate: Document<'static, String> = Document {
            definitions: vec![
                Definition::SchemaDefinition(SchemaDefinition::default()),
                Definition::SchemaDefinition(SchemaDefinition::default()),
            ],
        };

        assert_ne!(hash_of(&single), hash_of(&duplicate));
    }
}
