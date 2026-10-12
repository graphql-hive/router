//! `content` from `e2e/src/issues/supergraph.1669.graphql`: owns the favorites page, an entity
//! keyed by `__typename` alone.

use async_graphql::{EmptyMutation, EmptySubscription, Object, Schema, SimpleObject};

#[derive(SimpleObject)]
pub struct FavoritesPage {
    title: Option<String>,
}

impl FavoritesPage {
    fn new() -> Self {
        FavoritesPage {
            title: Some("My favorites".to_string()),
        }
    }
}

pub struct Query;

#[Object]
impl Query {
    async fn favorites_page(&self) -> Option<FavoritesPage> {
        Some(FavoritesPage::new())
    }

    /// `@key(fields: "__typename")`: the representation has nothing but `__typename`.
    #[graphql(entity)]
    async fn find_favorites_page(
        &self,
        #[graphql(key, name = "__typename")] _typename: String,
    ) -> FavoritesPage {
        FavoritesPage::new()
    }
}

pub fn get_subgraph() -> Schema<Query, EmptyMutation, EmptySubscription> {
    Schema::build(Query, EmptyMutation, EmptySubscription)
        .enable_federation()
        .finish()
}
