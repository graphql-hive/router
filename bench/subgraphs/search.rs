//! `search` from `e2e/src/issues/supergraph.1309.graphql`: lists the pets up for adoption.

use async_graphql::{
    EmptyMutation, EmptySubscription, Interface, Object, Schema, SimpleObject, ID,
};
use lazy_static::lazy_static;

lazy_static! {
    static ref LISTINGS: Vec<Listing> = vec![
        Listing {
            id: ID::from("l1"),
            pet: Some(Animal::Cat(Cat { id: ID::from("c1") })),
        },
        Listing {
            id: ID::from("l2"),
            pet: Some(Animal::Dog(Dog { id: ID::from("d1") })),
        },
        Listing {
            id: ID::from("l3"),
            pet: Some(Animal::Bird(Bird { id: ID::from("b1") })),
        },
    ];
}

#[derive(Interface, Clone)]
#[graphql(field(name = "id", ty = "&ID"))]
pub enum Animal {
    Cat(Cat),
    Dog(Dog),
    Bird(Bird),
}

#[derive(SimpleObject, Clone)]
pub struct Cat {
    id: ID,
}

#[derive(SimpleObject, Clone)]
pub struct Dog {
    id: ID,
}

#[derive(SimpleObject, Clone)]
pub struct Bird {
    id: ID,
}

#[derive(SimpleObject, Clone)]
pub struct Listing {
    id: ID,
    pet: Option<Animal>,
}

pub struct Query;

#[Object]
impl Query {
    async fn listings(&self) -> Option<Vec<Listing>> {
        Some(LISTINGS.clone())
    }
}

pub fn get_subgraph() -> Schema<Query, EmptyMutation, EmptySubscription> {
    Schema::build(Query, EmptyMutation, EmptySubscription)
        .enable_federation()
        .finish()
}
