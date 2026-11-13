//! Semantic indexing database, physically separate from DVR `index.sqlite`.

mod store;

pub use store::SemanticSqliteStore;
