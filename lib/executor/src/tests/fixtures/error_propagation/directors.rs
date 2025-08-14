use async_graphql::{EmptyMutation, EmptySubscription, Object, Schema, SimpleObject, ID};

#[derive(SimpleObject, Clone)]
#[graphql(extends)]
pub struct Movie {
    #[graphql(external)]
    id: String,
    director: Director,
}
pub struct Query;

#[derive(SimpleObject, Clone)]
pub struct Director {
    id: String,
    name: String,
}

#[Object(extends = true)]
impl Query {
    #[graphql(entity)]
    async fn movie_with_director(&self, id: ID) -> Result<Movie, async_graphql::Error> {
        // Throw on purpose
        if id == ID("2".to_string()) {
            Err(async_graphql::Error::new(
                "Director not found for movie with id 2",
            ))
        } else {
            Ok(Movie {
                id: id.to_string(),
                director: Director {
                    id: "1".to_string(),
                    name: "Christopher Nolan".to_string(),
                },
            })
        }
    }
}

pub fn get_subgraph() -> Schema<Query, EmptyMutation, EmptySubscription> {
    Schema::build(Query, EmptyMutation, EmptySubscription)
        .enable_federation()
        .finish()
}
