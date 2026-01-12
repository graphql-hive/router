use super::rules::ValidationRule;
use super::utils::ValidationError;
use super::validate::validate;
use super::validate::ValidationPlan;

#[cfg(test)]
pub static INTROSPECTION_SCHEMA: &str = "
directive @skip(if: Boolean!) on FIELD | FRAGMENT_SPREAD | INLINE_FRAGMENT
directive @include(if: Boolean!) on FIELD | FRAGMENT_SPREAD | INLINE_FRAGMENT

scalar Boolean
scalar Float
scalar Int
scalar ID
scalar String

type Query {
  __schema: __Schema!
  __type(name: String!): __Type
}

type __Schema {
  types: [__Type!]!
  queryType: __Type!
  mutationType: __Type
  subscriptionType: __Type
  directives: [__Directive!]!
}

type __Type {
  kind: __TypeKind!
  name: String
  description: String

  # OBJECT and INTERFACE only
  fields(includeDeprecated: Boolean = false): [__Field!]

  # OBJECT only
  interfaces: [__Type!]

  # INTERFACE and UNION only
  possibleTypes: [__Type!]

  # ENUM only
  enumValues(includeDeprecated: Boolean = false): [__EnumValue!]

  # INPUT_OBJECT only
  inputFields: [__InputValue!]

  # NON_NULL and LIST only
  ofType: __Type
}

type __Field {
  name: String!
  description: String
  args: [__InputValue!]!
  type: __Type!
  isDeprecated: Boolean!
  deprecationReason: String
}

type __InputValue {
  name: String!
  description: String
  type: __Type!
  defaultValue: String
}

type __EnumValue {
  name: String!
  description: String
  isDeprecated: Boolean!
  deprecationReason: String
}

enum __TypeKind {
  SCALAR
  OBJECT
  INTERFACE
  UNION
  ENUM
  INPUT_OBJECT
  LIST
  NON_NULL
}

type __Directive {
  name: String!
  description: String
  locations: [__DirectiveLocation!]!
  args: [__InputValue!]!
}

enum __DirectiveLocation {
  QUERY
  MUTATION
  SUBSCRIPTION
  FIELD
  FRAGMENT_DEFINITION
  FRAGMENT_SPREAD
  INLINE_FRAGMENT
  SCHEMA
  SCALAR
  OBJECT
  FIELD_DEFINITION
  ARGUMENT_DEFINITION
  INTERFACE
  UNION
  ENUM
  ENUM_VALUE
  INPUT_OBJECT
  INPUT_FIELD_DEFINITION
}";

#[cfg(test)]
pub static TEST_SCHEMA: &str = "
interface Mammal {
  mother: Mammal
  father: Mammal
}
interface Pet {
  name(surname: Boolean): String
}
interface Canine implements Mammal {
  name(surname: Boolean): String
  mother: Canine
  father: Canine
}
enum DogCommand {
  SIT
  HEEL
  DOWN
}
type Dog implements Pet & Mammal & Canine {
  name(surname: Boolean): String
  nickname: String
  barkVolume: Int
  barks: Boolean
  doesKnowCommand(dogCommand: DogCommand): Boolean
  isHouseTrained(atOtherHomes: Boolean = true): Boolean
  isAtLocation(x: Int, y: Int): Boolean
  mother: Dog
  father: Dog
}
type Cat implements Pet {
  name(surname: Boolean): String
  nickname: String
  meows: Boolean
  meowsVolume: Int
  furColor: FurColor
}
union CatOrDog = Cat | Dog
type Human {
  name(surname: Boolean): String
  pets: [Pet]
  relatives: [Human]
}
enum FurColor {
  BROWN
  BLACK
  TAN
  SPOTTED
  NO_FUR
  UNKNOWN
}
input ComplexInput {
  requiredField: Boolean!
  nonNullField: Boolean! = false
  intField: Int
  stringField: String
  booleanField: Boolean
  stringListField: [String]
}
type ComplicatedArgs {
  # TODO List
  # TODO Coercion
  # TODO NotNulls
  intArgField(intArg: Int): String
  nonNullIntArgField(nonNullIntArg: Int!): String
  stringArgField(stringArg: String): String
  booleanArgField(booleanArg: Boolean): String
  enumArgField(enumArg: FurColor): String
  floatArgField(floatArg: Float): String
  idArgField(idArg: ID): String
  stringListArgField(stringListArg: [String]): String
  stringListNonNullArgField(stringListNonNullArg: [String!]): String
  complexArgField(complexArg: ComplexInput): String
  multipleReqs(req1: Int!, req2: Int!): String
  nonNullFieldWithDefault(arg: Int! = 0): String
  multipleOpts(opt1: Int = 0, opt2: Int = 0): String
  multipleOptAndReq(req1: Int!, req2: Int!, opt1: Int = 0, opt2: Int = 0): String
}
type QueryRoot {
  human(id: ID): Human
  dog: Dog
  cat: Cat
  pet: Pet
  catOrDog: CatOrDog
  complicatedArgs: ComplicatedArgs
}

type SubscriptionRoot {
  fieldB: String
}

type MutationRoot {
  fieldB: String
}

schema {
  subscription: SubscriptionRoot
  mutation: MutationRoot
  query: QueryRoot
}
directive @onField on FIELD
directive @onQuery on QUERY
directive @onMutation on MUTATION
directive @onSubscription on SUBSCRIPTION
directive @onFragmentDefinition on FRAGMENT_DEFINITION
directive @onFragmentSpread on FRAGMENT_SPREAD
directive @onInlineFragment on INLINE_FRAGMENT
directive @testDirective on FIELD | FRAGMENT_DEFINITION 

# doesn't work see https://github.com/graphql-rust/graphql-parser/issues/60
# directive @onVariableDefinition on VARIABLE_DEFINITION

directive @repeatable repeatable on FIELD | FRAGMENT_DEFINITION
";

#[cfg(test)]
pub fn create_plan_from_rule(rule: Box<dyn ValidationRule>) -> ValidationPlan {
    let mut rules = Vec::new();
    rules.push(rule);

    ValidationPlan { rules }
}

#[cfg(test)]
pub fn get_messages(validation_errors: &[ValidationError]) -> Vec<&String> {
    validation_errors
        .iter()
        .map(|m| &m.message)
        .collect::<Vec<&String>>()
}

#[cfg(test)]
pub fn test_operation_without_schema<'a>(
    operation: &'a str,
    plan: &'a mut ValidationPlan,
) -> Vec<ValidationError> {
    let schema_ast = crate::parser::parse_schema(
        "
type Query {
  dummy: String
}
",
    )
    .expect("Failed to parse schema");

    let operation_ast = crate::parser::parse_query(operation).unwrap().into_static();

    validate(&schema_ast, &operation_ast, plan)
}

#[cfg(test)]
fn string_to_static_str(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

#[cfg(test)]
pub fn test_operation_with_schema<'a>(
    operation: &'a str,
    schema: &'a str,
    plan: &'a ValidationPlan,
) -> Vec<ValidationError> {
    let schema_clone = string_to_static_str(schema.to_string() + INTROSPECTION_SCHEMA);
    let schema_ast = crate::parser::parse_schema(schema_clone).expect("Failed to parse schema");

    let operation_ast = crate::parser::parse_query(operation).unwrap().into_static();

    validate(&schema_ast, &operation_ast, plan)
}
