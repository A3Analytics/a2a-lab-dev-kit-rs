//! Catalog-backed workflow provider.

use std::sync::Arc;

use crate::catalog::{AssetCatalogProvider, BindingRole, ListBindingsRequest};
use crate::error::SdkError;
use crate::id::{RunId, WorkflowId};
use crate::industrial::LiveSource;
use crate::industrial::logs::find_binding;
use crate::json_object::JsonObject;
use crate::page::{Page, PageRequest};
use crate::workflows::{
    GetWorkflowStatusRequest, ListWorkflowsRequest, StartWorkflowRequest, WorkflowDefinition,
    WorkflowProvider, WorkflowRun,
};

/// Lists workflows from the catalog and runs them through the bound protocol.
pub struct IndustrialWorkflows<C, L> {
    catalog: Arc<C>,
    live: Arc<L>,
}

impl<C, L> IndustrialWorkflows<C, L> {
    /// Creates a provider over a shared catalog and live source.
    #[must_use]
    pub const fn new(catalog: Arc<C>, live: Arc<L>) -> Self {
        Self { catalog, live }
    }
}

impl<C, L> WorkflowProvider for IndustrialWorkflows<C, L>
where
    C: AssetCatalogProvider,
    L: LiveSource,
{
    async fn list_workflows(
        &self,
        request: ListWorkflowsRequest,
    ) -> Result<Page<WorkflowDefinition>, SdkError> {
        let bindings = self
            .catalog
            .list_bindings(ListBindingsRequest {
                page: PageRequest::new(None, crate::page::MAX_PAGE_LIMIT)?,
            })
            .await?;
        let mut workflows = Vec::new();
        for binding in bindings.items() {
            if binding.role() != BindingRole::Workflow {
                continue;
            }
            workflows.push(WorkflowDefinition {
                id: WorkflowId::new(binding.lab_id())?,
                name: binding.lab_id().to_owned(),
                description: binding.semantic_id().as_str().to_owned(),
                asset_id: Some(binding.asset_key().as_str().to_owned()),
                semantic_id: Some(binding.semantic_id().as_str().to_owned()),
            });
        }
        crate::page::slice_page(&workflows, &request.page)
    }

    async fn start(&self, request: StartWorkflowRequest) -> Result<WorkflowRun, SdkError> {
        let binding = find_binding(
            &*self.catalog,
            request.workflow_id.as_str(),
            BindingRole::Workflow,
        )
        .await?;
        let run = self
            .live
            .start(
                binding.endpoint(),
                request.workflow_id.as_str(),
                request.input.clone(),
            )
            .await?;
        Ok(WorkflowRun {
            id: RunId::new(run.id)?,
            workflow_id: request.workflow_id,
            state: run.state,
            input: request.input,
            message: run.message,
        })
    }

    async fn status(&self, request: GetWorkflowStatusRequest) -> Result<WorkflowRun, SdkError> {
        let run = self.live.status(request.run_id.as_str()).await?;
        Ok(WorkflowRun {
            id: request.run_id,
            workflow_id: WorkflowId::new(run.workflow_id)?,
            state: run.state,
            input: JsonObject::empty(),
            message: run.message,
        })
    }
}
