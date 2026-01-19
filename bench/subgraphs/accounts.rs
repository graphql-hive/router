use async_graphql::{
    ComplexObject, EmptyMutation, EmptySubscription, ID, Interface, Object, Schema, SimpleObject
};
use lazy_static::lazy_static;

lazy_static! {
    static ref USERS: Vec<User> = vec![
        User {
            id: ID("1".to_string()),
            name: Some("Uri Goldshtein".to_string()),
            username: Some("urigo".to_string()),
            birthday: Some(1234567890),
        },
        User {
            id: ID("2".to_string()),
            name: Some("Dotan Simha".to_string()),
            username: Some("dotansimha".to_string()),
            birthday: Some(1234567890),
        },
        User {
            id: ID("3".to_string()),
            name: Some("Kamil Kisiela".to_string()),
            username: Some("kamilkisiela".to_string()),
            birthday: Some(1234567890),
        },
        User {
            id: ID("4".to_string()),
            name: Some("Arda Tanrikulu".to_string()),
            username: Some("ardatan".to_string()),
            birthday: Some(1234567890),
        },
        User {
            id: ID("5".to_string()),
            name: Some("Gil Gardosh".to_string()),
            username: Some("gilgardosh".to_string()),
            birthday: Some(1234567890),
        },
        User {
            id: ID("6".to_string()),
            name: Some("Laurin Quast".to_string()),
            username: Some("laurin".to_string()),
            birthday: Some(1234567890),
        }
    ];
}

#[derive(Interface, Clone)]
#[allow(clippy::duplicated_attributes)] // async_graphql needs `ty` "duplicated"
#[graphql(
    field(name = "url", ty = "String"),
    field(name = "handle", ty = "String")
)]
pub enum SocialAccount {
    TwitterAccount(TwitterAccount),
    GitHubAccount(GitHubAccount),
}

#[derive(SimpleObject, Clone)]
pub struct TwitterAccount {
    url: String,
    handle: String,
    followers: i32,
}

#[derive(SimpleObject, Clone)]
pub struct GitHubAccount {
    url: String,
    handle: String,
    repo_count: i32,
}

#[derive(SimpleObject, Clone)]
#[graphql(complex)]
pub struct User {
    id: ID,
    name: Option<String>,
    username: Option<String>,
    birthday: Option<i32>,
}

#[ComplexObject]
impl User {
    async fn social_accounts(&self) -> Vec<SocialAccount> {
        vec![
            SocialAccount::TwitterAccount(TwitterAccount {
                url: format!(
                    "https://twitter.com/{}",
                    self.username.as_ref().unwrap_or(&"unknown".to_string())
                ),
                handle: format!(
                    "@{}",
                    self.username.as_ref().unwrap_or(&"unknown".to_string())
                ),
                followers: 1000,
            }),
            SocialAccount::GitHubAccount(GitHubAccount {
                url: format!(
                    "https://github.com/{}",
                    self.username.as_ref().unwrap_or(&"unknown".to_string())
                ),
                handle: self
                    .username
                    .as_ref()
                    .unwrap_or(&"unknown".to_string())
                    .clone(),
                repo_count: 42,
            }),
        ]
    }
}

impl User {
    fn me() -> User {
        USERS[0].clone()
    }
}

pub struct Query;

#[Object(extends = true)]
impl Query {
    async fn me(&self) -> Option<User> {
        Some(User::me())
    }

    async fn user(&self, id: ID) -> Option<User> {
        USERS.iter().find(|user| user.id == id).cloned()
    }

    async fn users(&self) -> Option<Vec<Option<User>>> {
        Some(USERS.iter().map(|user| Some(user.clone())).collect())
    }

    #[graphql(entity)]
    async fn find_user_by_id(&self, id: ID) -> User {
        USERS.iter().find(|user| user.id == id).cloned().unwrap()
    }
}

pub fn get_subgraph() -> Schema<Query, EmptyMutation, EmptySubscription> {
    Schema::build(Query, EmptyMutation, EmptySubscription)
        .enable_federation()
        .finish()
}
