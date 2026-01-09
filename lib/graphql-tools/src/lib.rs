//! graphql-tools
//! ==============
//!
//! This library implements tooling around GraphQL for Rust libraries.
//! Most of the tools are based on `trait`s and `struct`s implemented in `graphql_parser` crate.
//!

pub mod ast;

pub mod static_graphql {
    macro_rules! static_graphql {
    ($m:ident, $m2:ident, {$($n:ident,)*}) => {
        pub mod $m {
            use crate::parser::$m2 as $m;
            pub use $m::*;
            $(
                pub type $n = $m::$n<'static, String>;
            )*
        }
    };
  }

    static_graphql!(query, query, {
      Document, Value, OperationDefinition, InlineFragment, TypeCondition,
      FragmentSpread, Field, Selection, SelectionSet, FragmentDefinition,
      Directive, VariableDefinition, Type, Query, Definition, Subscription, Mutation,
    });
    static_graphql!(schema, schema, {
      Field, Directive, InterfaceType, ObjectType, Value, TypeDefinition,
      EnumType, Type, Document, ScalarType, InputValue, DirectiveDefinition,
      UnionType, InputObjectType, EnumValue, SchemaDefinition,
    });
}

pub mod introspection;

pub mod parser;
pub mod validation;
