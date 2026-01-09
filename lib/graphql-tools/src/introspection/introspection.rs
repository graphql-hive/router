use std::io;

use serde::{Deserialize, Serialize};
use serde_json::{Result, Value};

#[derive(Serialize, Deserialize, Debug)]
pub struct IntrospectionQuery {
    pub __schema: IntrospectionSchema,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IntrospectionScalarType {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "specifiedByURL")]
    pub specified_by_url: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IntrospectionInputValue {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "defaultValue")]
    pub default_value: Option<Value>,
    #[serde(rename = "isDeprecated")]
    pub is_deprecated: Option<bool>,
    #[serde(rename = "deprecationReason")]
    pub deprecation_reason: Option<String>,
    #[serde(rename = "type")]
    pub type_ref: Option<IntrospectionInputTypeRef>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "kind")]
pub struct IntrospectionField {
    pub name: String,
    pub description: Option<String>,
    pub args: Vec<IntrospectionInputValue>,
    #[serde(rename = "isDeprecated")]
    pub is_deprecated: Option<bool>,
    #[serde(rename = "deprecationReason")]
    pub deprecation_reason: Option<String>,
    #[serde(rename = "type")]
    pub type_ref: IntrospectionOutputTypeRef,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IntrospectionObjectType {
    pub name: String,
    pub description: Option<String>,
    pub fields: Vec<IntrospectionField>,
    pub interfaces: Vec<IntrospectionNamedTypeRef>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IntrospectionInterfaceType {
    pub name: String,
    pub description: Option<String>,
    pub fields: Vec<IntrospectionField>,
    pub interfaces: Option<Vec<IntrospectionNamedTypeRef>>,
    #[serde(rename = "possibleTypes")]
    pub possible_types: Vec<IntrospectionNamedTypeRef>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IntrospectionUnionType {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "possibleTypes")]
    pub possible_types: Vec<IntrospectionNamedTypeRef>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IntrospectionEnumValue {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "isDeprecated")]
    pub is_deprecated: Option<bool>,
    #[serde(rename = "deprecationReason")]
    pub deprecation_reason: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IntrospectionEnumType {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "enumValues")]
    pub enum_values: Vec<IntrospectionEnumValue>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IntrospectionInputObjectType {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "inputFields")]
    pub input_fields: Vec<IntrospectionInputValue>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "kind")]
pub enum IntrospectionType {
    SCALAR(IntrospectionScalarType),
    OBJECT(IntrospectionObjectType),
    INTERFACE(IntrospectionInterfaceType),
    UNION(IntrospectionUnionType),
    ENUM(IntrospectionEnumType),
    INPUT_OBJECT(IntrospectionInputObjectType),
}

impl IntrospectionType {
    pub fn name(&self) -> &String {
        match &self {
            IntrospectionType::ENUM(e) => &e.name,
            IntrospectionType::OBJECT(o) => &o.name,
            IntrospectionType::INPUT_OBJECT(io) => &io.name,
            IntrospectionType::INTERFACE(i) => &i.name,
            IntrospectionType::SCALAR(s) => &s.name,
            IntrospectionType::UNION(u) => &u.name,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "kind")]
pub enum IntrospectionInputType {
    SCALAR(IntrospectionScalarType),
    ENUM(IntrospectionEnumType),
    INPUT_OBJECT(IntrospectionInputObjectType),
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "kind")]
pub enum IntrospectionOutputType {
    SCALAR(IntrospectionScalarType),
    OBJECT(IntrospectionObjectType),
    INTERFACE(IntrospectionInterfaceType),
    UNION(IntrospectionUnionType),
    ENUM(IntrospectionEnumType),
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IntrospectionNamedTypeRef {
    pub name: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "kind")]
pub enum IntrospectionOutputTypeRef {
    SCALAR(IntrospectionNamedTypeRef),
    LIST {
        #[serde(rename = "ofType")]
        of_type: Option<Box<IntrospectionOutputTypeRef>>,
    },
    NON_NULL {
        #[serde(rename = "ofType")]
        of_type: Option<Box<IntrospectionOutputTypeRef>>,
    },
    ENUM(IntrospectionNamedTypeRef),
    INPUT_OBJECT(IntrospectionNamedTypeRef),
    UNION(IntrospectionNamedTypeRef),
    OBJECT(IntrospectionNamedTypeRef),
    INTERFACE(IntrospectionNamedTypeRef),
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "kind")]
pub enum IntrospectionInputTypeRef {
    LIST {
        #[serde(rename = "ofType")]
        of_type: Option<Box<IntrospectionInputTypeRef>>,
    },
    NON_NULL {
        #[serde(rename = "ofType")]
        of_type: Option<Box<IntrospectionInputTypeRef>>,
    },
    SCALAR(IntrospectionNamedTypeRef),
    ENUM(IntrospectionNamedTypeRef),
    INPUT_OBJECT(IntrospectionNamedTypeRef),
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IntrospectionSchema {
    pub description: Option<String>,
    #[serde(rename = "queryType")]
    pub query_type: IntrospectionNamedTypeRef,
    #[serde(rename = "mutationType")]
    pub mutation_type: Option<IntrospectionNamedTypeRef>,
    #[serde(rename = "subscriptionType")]
    pub subscription_type: Option<IntrospectionNamedTypeRef>,
    pub types: Vec<IntrospectionType>,
    pub directives: Vec<IntrospectionDirective>,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum DirectiveLocation {
    QUERY,
    MUTATION,
    SUBSCRIPTION,
    FIELD,
    FRAGMENT_DEFINITION,
    FRAGMENT_SPREAD,
    INLINE_FRAGMENT,
    VARIABLE_DEFINITION,
    /** Type System Definitions */
    SCHEMA,
    SCALAR,
    OBJECT,
    FIELD_DEFINITION,
    ARGUMENT_DEFINITION,
    INTERFACE,
    UNION,
    ENUM,
    ENUM_VALUE,
    INPUT_OBJECT,
    INPUT_FIELD_DEFINITION,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IntrospectionDirective {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "isRepeatable")]
    pub is_repeatable: Option<bool>,
    pub locations: Vec<DirectiveLocation>,
    pub args: Vec<IntrospectionInputValue>,
}

pub fn parse_introspection_from_string(input: &str) -> Result<IntrospectionQuery> {
    serde_json::from_str(input)
}

pub fn parse_introspection<R>(input: R) -> Result<IntrospectionQuery>
where
    R: io::Read,
{
    serde_json::from_reader::<R, IntrospectionQuery>(input)
}

#[test]
fn test_product_introspection() {
    use std::fs::File;
    let json_file = File::open("./src/introspection/test_files/product_introspection.json")
        .expect("failed to open json file");
    parse_introspection(json_file).expect("failed to parse introspection json");
}

#[test]
fn test_github_introspection() {
    use std::fs::File;
    let json_file = File::open("./src/introspection/test_files/github_introspection.json")
        .expect("failed to open json file");
    parse_introspection(json_file).expect("failed to parse introspection json");
}

#[test]
fn test_shopify_introspection() {
    use std::fs::File;
    let json_file = File::open("./src/introspection/test_files/shopify_introspection.json")
        .expect("failed to open json file");
    parse_introspection(json_file).expect("failed to parse introspection json");
}
