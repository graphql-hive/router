use async_graphql::{
    ComplexObject, EmptyMutation, EmptySubscription, Object, Schema, SimpleObject, ID,
};
use lazy_static::lazy_static;

lazy_static! {
    static ref INVENTORY: Vec<Product> = vec![
        Product {
            upc: "1".to_string(),
            in_stock: Some(true),
            price: None,
            weight: None,
        },
        Product {
            upc: "2".to_string(),
            in_stock: Some(false),
            price: None,
            weight: None,
        },
        Product {
            upc: "3".to_string(),
            in_stock: Some(false),
            price: None,
            weight: None,
        },
        Product {
            upc: "4".to_string(),
            in_stock: Some(false),
            price: None,
            weight: None,
        },
        Product {
            upc: "5".to_string(),
            in_stock: Some(true),
            price: None,
            weight: None,
        },
        Product {
            upc: "6".to_string(),
            in_stock: Some(true),
            price: None,
            weight: None,
        },
        Product {
            upc: "7".to_string(),
            in_stock: Some(true),
            price: None,
            weight: None,
        },
        Product {
            upc: "8".to_string(),
            in_stock: Some(false),
            price: None,
            weight: None,
        },
        Product {
            upc: "9".to_string(),
            in_stock: Some(true),
            price: None,
            weight: None,
        }
    ];
}

#[derive(SimpleObject, Clone)]
#[graphql(extends, complex)]
pub struct Product {
    #[graphql(external)]
    upc: String,
    #[graphql(external)]
    weight: Option<i64>,
    #[graphql(external)]
    price: Option<i64>,
    in_stock: Option<bool>,
}

#[ComplexObject]
impl Product {
    #[graphql(requires = "price weight")]
    pub async fn shipping_estimate(&self) -> Option<i64> {
        if let Some(price) = self.price {
            if price > 1000 {
                return Some(0);
            }

            if let Some(weight) = self.weight {
                return Some(weight / 2);
            }
        }

        None
    }
}

pub struct Query;

#[Object(extends = true)]
impl Query {
    #[graphql(entity)]
    async fn find_product_by_id(&self, upc: ID) -> Product {
        INVENTORY
            .iter()
            .find(|product| product.upc == upc.to_string())
            .unwrap()
            .clone()
    }
}

pub fn get_subgraph() -> Schema<Query, EmptyMutation, EmptySubscription> {
    Schema::build(Query, EmptyMutation, EmptySubscription)
        .enable_federation()
        .finish()
}
