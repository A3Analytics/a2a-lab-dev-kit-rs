//! Semantic asset catalog shared by industrial protocol clients.

mod id;
mod model;
mod provider;

pub use id::{AssetKey, SemanticId, SemanticKind};
pub use model::{
    Asset, Binding, BindingRole, Endpoint, ListAssetsRequest, ListBindingsRequest,
    OpcUaIdentityKind, ProtocolKind, SecurityMode,
};
pub use provider::AssetCatalogProvider;
