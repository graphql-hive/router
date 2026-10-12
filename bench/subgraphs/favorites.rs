//! `favorites` from `e2e/src/issues/supergraph.1669.graphql`: has the number of favorite pets,
//! but here it can't resolve the page, like for a logged-out user, and returns `null` for it.

use async_graphql::{EmptyMutation, EmptySubscription, Object, Schema, SimpleObject};

#[derive(SimpleObject)]
pub struct FavoritesPage {
    pet_count: Option<i32>,
}

pub struct Query;

#[Object]
impl Query {
    /// `@key(fields: "__typename")`: the representation has nothing but `__typename`.
    #[graphql(entity)]
    async fn find_favorites_page(
        &self,
        #[graphql(key, name = "__typename")] _typename: String,
    ) -> Option<FavoritesPage> {
        None
    }
}

pub fn get_subgraph() -> Schema<Query, EmptyMutation, EmptySubscription> {
    Schema::build(Query, EmptyMutation, EmptySubscription)
        .enable_federation()
        .finish()
}
