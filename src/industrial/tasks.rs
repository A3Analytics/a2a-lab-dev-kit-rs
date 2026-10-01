//! Catalog-backed task provider.

use std::sync::Arc;

use crate::catalog::{AssetCatalogProvider, BindingRole, ListBindingsRequest};
use crate::error::A2aLabError;
use crate::id::{RunId, TaskId};
use crate::industrial::LiveSource;
use crate::industrial::logs::find_binding;
use crate::json_object::JsonObject;
use crate::page::{Page, PageRequest};
use crate::tasks::{
    GetTaskStatusRequest, ListTasksRequest, StartTaskRequest, TaskDefinition, TaskProvider, TaskRun,
};

/// Lists tasks from the catalog and runs them through the bound protocol.
pub struct IndustrialTasks<C, L> {
    catalog: Arc<C>,
    live: Arc<L>,
}

impl<C, L> IndustrialTasks<C, L> {
    /// Creates a provider over a shared catalog and live source.
    #[must_use]
    pub const fn new(catalog: Arc<C>, live: Arc<L>) -> Self {
        Self { catalog, live }
    }
}

impl<C, L> TaskProvider for IndustrialTasks<C, L>
where
    C: AssetCatalogProvider,
    L: LiveSource,
{
    async fn list_tasks(
        &self,
        request: ListTasksRequest,
    ) -> Result<Page<TaskDefinition>, A2aLabError> {
        let bindings = self
            .catalog
            .list_bindings(ListBindingsRequest {
                page: PageRequest::new(None, crate::page::MAX_PAGE_LIMIT)?,
            })
            .await?;
        let mut tasks = Vec::new();
        for binding in bindings.items() {
            if binding.role() != BindingRole::Task {
                continue;
            }
            tasks.push(TaskDefinition {
                id: TaskId::new(binding.lab_id())?,
                name: binding.lab_id().to_owned(),
                description: binding.semantic_id().as_str().to_owned(),
                asset_id: Some(binding.asset_key().as_str().to_owned()),
                semantic_id: Some(binding.semantic_id().as_str().to_owned()),
            });
        }
        crate::page::slice_page(&tasks, &request.page)
    }

    async fn start(&self, request: StartTaskRequest) -> Result<TaskRun, A2aLabError> {
        let binding =
            find_binding(&*self.catalog, request.task_id.as_str(), BindingRole::Task).await?;
        let run = self
            .live
            .start(
                binding.endpoint(),
                request.task_id.as_str(),
                request.input.clone(),
            )
            .await?;
        Ok(TaskRun {
            id: RunId::new(run.id)?,
            task_id: request.task_id,
            state: run.state,
            input: request.input,
            message: run.message,
        })
    }

    async fn status(&self, request: GetTaskStatusRequest) -> Result<TaskRun, A2aLabError> {
        let run = self.live.status(request.id.as_str()).await?;
        Ok(TaskRun {
            id: request.id,
            task_id: TaskId::new(run.task_id)?,
            state: run.state,
            input: JsonObject::empty(),
            message: run.message,
        })
    }
}
