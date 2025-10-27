use graphql_parser::query::Text;
use graphql_parser::schema::*;

use crate::{
    federation_spec::{
        definitions::{
            CorePurposesEnum, JoinFieldSetScalar, JoinGraphEnum, LinkImportScalar, LinkPurposeEnum,
        },
        directives::{
            CoreDirective, InaccessibleDirective, JoinEnumValueDirective, JoinFieldDirective,
            JoinGraphDirective, JoinImplementsDirective, JoinTypeDirective,
            JoinUnionMemberDirective, LinkDirective, TagDirective,
        },
        join_owner::JoinOwnerDirective,
    },
    utils::schema_transformer::{SchemaTransformer, TransformedValue},
};

// directive @inaccessible on FIELD_DEFINITION | OBJECT | INTERFACE | UNION | ENUM | ENUM_VALUE | SCALAR | INPUT_OBJECT | INPUT_FIELD_DEFINITION | ARGUMENT_DEFINITION
pub(crate) struct StripSchemaInternals;

static DIRECTIVES_TO_STRIP: [&str; 11] = [
    JoinTypeDirective::NAME,
    JoinEnumValueDirective::NAME,
    JoinFieldDirective::NAME,
    JoinImplementsDirective::NAME,
    JoinUnionMemberDirective::NAME,
    JoinGraphDirective::NAME,
    JoinOwnerDirective::NAME,
    LinkDirective::NAME,
    TagDirective::NAME,
    InaccessibleDirective::NAME,
    CoreDirective::NAME,
];

static DEFINITIONS_TO_STRIP: [&str; 5] = [
    LinkPurposeEnum::NAME,
    LinkImportScalar::NAME,
    JoinGraphEnum::NAME,
    JoinFieldSetScalar::NAME,
    CorePurposesEnum::NAME,
];

impl StripSchemaInternals {
    pub fn strip_schema_internals(schema: &Document<'static, String>) -> Document<'static, String> {
        let mut transformer = StripSchemaInternals {};
        let result = transformer
            .transform_document(schema)
            .replace_or_else(|| schema.clone());

        result
    }

    pub(crate) fn filter_directives<'a, T: Text<'a> + Clone>(
        directives: &Vec<Directive<'a, T>>,
    ) -> Vec<Directive<'a, T>> {
        directives
            .iter()
            .filter(|d| !DIRECTIVES_TO_STRIP.contains(&d.name.as_ref()))
            .cloned()
            .collect()
    }
}

impl<'a, T: Text<'a> + Clone> SchemaTransformer<'a, T> for StripSchemaInternals {
    fn transform_directives(
        &mut self,
        directives: &Vec<Directive<'a, T>>,
    ) -> TransformedValue<Vec<Directive<'a, T>>> {
        let new_directives = StripSchemaInternals::filter_directives(directives);

        TransformedValue::Replace(new_directives)
    }

    fn transform_document(
        &mut self,
        document: &Document<'a, T>,
    ) -> TransformedValue<Document<'a, T>> {
        let new_doc = Document {
            definitions: document
                .definitions
                .iter()
                .filter(|def| match def {
                    Definition::DirectiveDefinition(directive) => {
                        !DIRECTIVES_TO_STRIP.contains(&directive.name.as_ref())
                    }
                    Definition::TypeDefinition(TypeDefinition::Object(obj)) => {
                        !DEFINITIONS_TO_STRIP.contains(&obj.name.as_ref())
                    }
                    Definition::TypeDefinition(TypeDefinition::Interface(interface)) => {
                        !DEFINITIONS_TO_STRIP.contains(&interface.name.as_ref())
                    }
                    Definition::TypeDefinition(TypeDefinition::Union(union)) => {
                        !DEFINITIONS_TO_STRIP.contains(&union.name.as_ref())
                    }
                    Definition::TypeDefinition(TypeDefinition::Enum(enm)) => {
                        !DEFINITIONS_TO_STRIP.contains(&enm.name.as_ref())
                    }
                    Definition::TypeDefinition(TypeDefinition::Scalar(scalar)) => {
                        !DEFINITIONS_TO_STRIP.contains(&scalar.name.as_ref())
                    }
                    Definition::TypeDefinition(TypeDefinition::InputObject(input)) => {
                        !DEFINITIONS_TO_STRIP.contains(&input.name.as_ref())
                    }
                    Definition::TypeExtension(_) => todo!("TypeExtension not implemented"),
                    Definition::SchemaDefinition(_) => true,
                })
                .cloned()
                .collect(),
        };

        self.default_transform_document(&new_doc)
    }
}

#[cfg(test)]
mod test {
    use insta::assert_debug_snapshot;

    use crate::utils::parsing::parse_schema;

    #[test]
    fn test_strip_schema_internals() {
        use crate::consumer_schema::strip_schema_internals::StripSchemaInternals;

        let schema = r#"
            schema
  @link(url: "https://specs.apollo.dev/link/v1.0")
  @link(url: "https://specs.apollo.dev/join/v0.3", for: EXECUTION)
  @link(url: "https://specs.apollo.dev/tag/v0.3")
  @link(url: "https://specs.apollo.dev/inaccessible/v0.2", for: SECURITY)
  @link(
    url: "https://myspecs.dev/myDirective/v1.0"
    import: ["@myDirective", { name: "@anotherDirective", as: "@hello" }]
  ) {
  query: Query
}

directive @join__enumValue(graph: join__Graph!) repeatable on ENUM_VALUE

directive @join__field(
  graph: join__Graph
  requires: join__FieldSet
  provides: join__FieldSet
  type: String
  external: Boolean
  override: String
  usedOverridden: Boolean
) repeatable on FIELD_DEFINITION | INPUT_FIELD_DEFINITION

directive @join__graph(name: String!, url: String!) on ENUM_VALUE

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

directive @tag(
  name: String!
) repeatable on FIELD_DEFINITION | OBJECT | INTERFACE | UNION | ARGUMENT_DEFINITION | SCALAR | ENUM | ENUM_VALUE | INPUT_OBJECT | INPUT_FIELD_DEFINITION | SCHEMA

directive @inaccessible on FIELD_DEFINITION | OBJECT | INTERFACE | UNION | ENUM | ENUM_VALUE | SCALAR | INPUT_OBJECT | INPUT_FIELD_DEFINITION | ARGUMENT_DEFINITION

enum join__Graph {
  INVENTORY @join__graph(name: "inventory", url: "")
  PANDAS @join__graph(name: "pandas", url: "")
  PRODUCTS @join__graph(name: "products", url: "")
  REVIEWS @join__graph(name: "reviews", url: "")
  USERS @join__graph(name: "users", url: "")
}

directive @myDirective(a: String!) on FIELD_DEFINITION

directive @hello on FIELD_DEFINITION

type Product implements ProductItf & SkuItf
  @join__type(graph: INVENTORY, key: "id")
  @join__type(graph: PRODUCTS, key: "id")
  @join__type(graph: PRODUCTS, key: "sku package")
  @join__type(graph: PRODUCTS, key: "sku variation { id }")
  @join__type(graph: REVIEWS, key: "id")
  @join__implements(graph: INVENTORY, interface: "ProductItf")
  @join__implements(graph: PRODUCTS, interface: "ProductItf")
  @join__implements(graph: PRODUCTS, interface: "SkuItf")
  @join__implements(graph: REVIEWS, interface: "ProductItf") {
  id: ID! @tag(name: "hi-from-products")
  dimensions: ProductDimension
    @join__field(graph: INVENTORY, external: true)
    @join__field(graph: PRODUCTS)
  delivery(zip: String): DeliveryEstimates
    @join__field(
      graph: INVENTORY
      requires: "dimensions{...on ProductDimension{size weight}}"
    )
  sku: String @join__field(graph: PRODUCTS)
  package: String @join__field(graph: PRODUCTS)
  variation: ProductVariation @join__field(graph: PRODUCTS)
  name: String @hello @join__field(graph: PRODUCTS)
  createdBy: User @join__field(graph: PRODUCTS)
  hidden: String @join__field(graph: PRODUCTS)
  reviewsScore: Float! @join__field(graph: REVIEWS, override: "products")
  oldField: String @join__field(graph: PRODUCTS)
  reviewsCount: Int! @join__field(graph: REVIEWS)
  reviews: [Review!]! @join__field(graph: REVIEWS)
}

type ProductDimension
  @join__type(graph: INVENTORY)
  @join__type(graph: PRODUCTS) {
  size: String
  weight: Float
}

type DeliveryEstimates @join__type(graph: INVENTORY) {
  estimatedDelivery: String
  fastestDelivery: String
}

type Query
  @join__type(graph: INVENTORY)
  @join__type(graph: PANDAS)
  @join__type(graph: PRODUCTS)
  @join__type(graph: REVIEWS)
  @join__type(graph: USERS) {
  allPandas: [Panda] @join__field(graph: PANDAS)
  panda(name: ID!): Panda @join__field(graph: PANDAS)
  allProducts: [ProductItf] @join__field(graph: PRODUCTS)
  product(id: ID!): ProductItf @join__field(graph: PRODUCTS)
  review(id: Int!): Review @join__field(graph: REVIEWS)
}

type Mutation
  @join__type(graph: REVIEWS)
{
  newRandomReview: Review
}

type Panda @join__type(graph: PANDAS) {
  name: ID!
  favoriteFood: String @tag(name: "nom-nom-nom")
}

type ProductVariation @join__type(graph: PRODUCTS) {
  id: ID!
  name: String
}

type User
  @join__type(graph: PRODUCTS, key: "email")
  @join__type(graph: USERS, key: "email") {
  email: ID! @tag(name: "test-from-users")
  totalProductsCreated: Int
  name: String @join__field(graph: USERS)
}

type Review @join__type(graph: REVIEWS) {
  id: Int!
  body: String!
}

interface ProductItf implements SkuItf
  @join__type(graph: INVENTORY)
  @join__type(graph: PRODUCTS)
  @join__type(graph: REVIEWS)
  @join__implements(graph: PRODUCTS, interface: "SkuItf") {
  id: ID!
  dimensions: ProductDimension
    @join__field(graph: INVENTORY)
    @join__field(graph: PRODUCTS)
  delivery(zip: String): DeliveryEstimates @join__field(graph: INVENTORY)
  sku: String @join__field(graph: PRODUCTS)
  name: String @join__field(graph: PRODUCTS)
  package: String @join__field(graph: PRODUCTS)
  variation: ProductVariation @join__field(graph: PRODUCTS)
  createdBy: User @join__field(graph: PRODUCTS)
  hidden: String @join__field(graph: PRODUCTS) @inaccessible
  oldField: String
    @join__field(graph: PRODUCTS)
    @deprecated(reason: "refactored out")
  reviewsCount: Int! @join__field(graph: REVIEWS)
  reviewsScore: Float! @join__field(graph: REVIEWS)
  reviews: [Review!]! @join__field(graph: REVIEWS)
}

interface SkuItf @join__type(graph: PRODUCTS) {
  sku: String
}

enum ShippingClass @join__type(graph: INVENTORY) @join__type(graph: PRODUCTS) {
  STANDARD @join__enumValue(graph: INVENTORY) @join__enumValue(graph: PRODUCTS)
  EXPRESS @join__enumValue(graph: INVENTORY) @join__enumValue(graph: PRODUCTS)
  OVERNIGHT @join__enumValue(graph: INVENTORY)
}
  "#;

        let schema = graphql_parser::parse_schema(schema).unwrap();
        let schema = StripSchemaInternals::strip_schema_internals(&schema);
        let schema_str = format!("{}", schema);

        assert_debug_snapshot!(schema_str);

        parse_schema(&schema_str);
    }
}
