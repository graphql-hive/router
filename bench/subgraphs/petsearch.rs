//! `petsearch` from `e2e/src/issues/supergraph.1669.graphql`: the search box of the favorites
//! page, which requires the number of favorite pets from `favorites`.

use async_graphql::{
    ComplexObject, EmptyMutation, EmptySubscription, Object, Schema, SimpleObject,
};

#[derive(SimpleObject)]
pub struct SearchBox {
    placeholder: Option<String>,
}

#[derive(SimpleObject)]
#[graphql(complex)]
pub struct FavoritesPage {
    #[graphql(external)]
    pet_count: Option<i32>,
}

#[ComplexObject]
impl FavoritesPage {
    #[graphql(requires = "petCount")]
    async fn search_box(&self) -> SearchBox {
        SearchBox {
            placeholder: Some("Find a pet".to_string()),
        }
    }
}

pub struct Query;

#[Object]
impl Query {
    /// `@key(fields: "__typename")`: the representation has `__typename`, and `petCount` for
    /// `searchBox`.
    #[graphql(entity)]
    async fn find_favorites_page(
        &self,
        #[graphql(key, name = "__typename")] _typename: String,
        pet_count: Option<i32>,
    ) -> FavoritesPage {
        FavoritesPage { pet_count }
    }
}

pub fn get_subgraph() -> Schema<Query, EmptyMutation, EmptySubscription> {
    Schema::build(Query, EmptyMutation, EmptySubscription)
        .enable_federation()
        .finish()
}
