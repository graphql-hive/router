//! `ranking` from `e2e/src/issues/supergraph.1309.graphql`: ranks listings by their pet, which
//! it gets from the other subgraphs through `@requires`.

use async_graphql::{
    ComplexObject, EmptyMutation, EmptySubscription, Json, Object, Schema, SimpleObject, ID,
};
use serde::{Deserialize, Serialize};

/// The listing's pet, as `rank` requires it:
/// `pet { __typename ... on Dog { tricks } ... on Cat { whiskers } }`.
#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "__typename")]
pub enum Pet {
    Cat { whiskers: Option<i32> },
    Dog { tricks: Option<i32> },
    Bird,
}

#[derive(SimpleObject, Clone)]
#[graphql(complex)]
pub struct Listing {
    id: ID,
    #[graphql(skip)]
    pet: Option<Pet>,
}

#[ComplexObject]
impl Listing {
    /// A cat ranks by its whiskers, a dog by its tricks, and a bird 0.
    #[graphql(requires = "pet { __typename ... on Dog { tricks } ... on Cat { whiskers } }")]
    async fn rank(&self) -> Option<f64> {
        match self.pet.as_ref()? {
            Pet::Cat { whiskers } => whiskers.map(f64::from),
            Pet::Dog { tricks } => tricks.map(f64::from),
            Pet::Bird => Some(1.0),
        }
    }
}

pub struct Query;

#[Object]
impl Query {
    #[graphql(entity)]
    async fn find_listing_by_id(&self, #[graphql(key)] id: ID, pet: Option<Json<Pet>>) -> Listing {
        Listing {
            id,
            pet: pet.map(|pet| pet.0),
        }
    }
}

pub fn get_subgraph() -> Schema<Query, EmptyMutation, EmptySubscription> {
    Schema::build(Query, EmptyMutation, EmptySubscription)
        .enable_federation()
        .finish()
}
