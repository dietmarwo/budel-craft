pub mod execution;
pub mod export;
pub mod geometry;
pub mod layout;
pub mod model;
pub mod runtime;
pub mod search;
pub mod validate;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
