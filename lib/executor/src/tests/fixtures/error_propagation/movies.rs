use async_graphql::{EmptyMutation, EmptySubscription, Object, Schema, SimpleObject, ID};

#[derive(SimpleObject, Clone)]
pub struct Movie {
    id: ID,
    name: String,
}
pub struct Query;

#[Object(extends = true)]
impl Query {
    async fn movie(&self, id: ID) -> Movie {
        // Simulate a movie fetch
        if id == ID("1".to_string()) {
            Movie {
                id: ID("1".to_string()),
                name: "Inception".to_string(),
            }
        } else {
            Movie {
                id: ID("2".to_string()),
                name: "Interstellar".to_string(),
            }
        }
    }
}

pub fn get_subgraph() -> Schema<Query, EmptyMutation, EmptySubscription> {
    Schema::build(Query, EmptyMutation, EmptySubscription)
        .enable_federation()
        .finish()
}
