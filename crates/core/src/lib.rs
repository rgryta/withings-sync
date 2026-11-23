//! Models, the connection pool and every SQL statement in the project.
//!
//! `store` owns all SQL and nothing outside it writes a query.

pub mod config;
pub mod db;
pub mod models;
pub mod store;
