//! `catalog` from `e2e/src/issues/supergraph.1309.graphql`: knows the details of every pet.

use async_graphql::{
    EmptyMutation, EmptySubscription, Interface, Object, Schema, SimpleObject, ID,
};

#[derive(Interface, Clone)]
#[graphql(
    field(name = "id", ty = "&ID"),
    field(name = "name", ty = "&Option<String>")
)]
pub enum Animal {
    Cat(Cat),
    Dog(Dog),
    Bird(Bird),
}

#[derive(SimpleObject, Clone)]
pub struct Cat {
    id: ID,
    name: Option<String>,
    whiskers: Option<i32>,
}

#[derive(SimpleObject, Clone)]
pub struct Dog {
    id: ID,
    name: Option<String>,
    tricks: Option<i32>,
}

#[derive(SimpleObject, Clone)]
pub struct Bird {
    id: ID,
    name: Option<String>,
}

fn find_animal(id: &ID) -> Option<Animal> {
    let name = |name: &str| Some(name.to_string());
    match id.as_str() {
        "c1" => Some(Animal::Cat(Cat {
            id: id.clone(),
            name: name("Tom"),
            whiskers: Some(12),
        })),
        "d1" => Some(Animal::Dog(Dog {
            id: id.clone(),
            name: name("Rex"),
            tricks: Some(3),
        })),
        "b1" => Some(Animal::Bird(Bird {
            id: id.clone(),
            name: name("Tweety"),
        })),
        _ => None,
    }
}

pub struct Query;

#[Object]
impl Query {
    /// `Animal` is an entity interface, so `catalog` finds out which pet an `Animal` is.
    #[graphql(entity)]
    async fn find_animal_by_id(&self, id: ID) -> Option<Animal> {
        find_animal(&id)
    }

    #[graphql(entity)]
    async fn find_cat_by_id(&self, id: ID) -> Option<Cat> {
        match find_animal(&id)? {
            Animal::Cat(cat) => Some(cat),
            _ => None,
        }
    }

    #[graphql(entity)]
    async fn find_dog_by_id(&self, id: ID) -> Option<Dog> {
        match find_animal(&id)? {
            Animal::Dog(dog) => Some(dog),
            _ => None,
        }
    }

    #[graphql(entity)]
    async fn find_bird_by_id(&self, id: ID) -> Option<Bird> {
        match find_animal(&id)? {
            Animal::Bird(bird) => Some(bird),
            _ => None,
        }
    }
}

pub fn get_subgraph() -> Schema<Query, EmptyMutation, EmptySubscription> {
    Schema::build(Query, EmptyMutation, EmptySubscription)
        .enable_federation()
        .finish()
}
