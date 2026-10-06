//! ObservablePropertyTest feature.

use std::sync::{Arc, Mutex};

use futures_util::stream;
use tokio::sync::Notify;
use tonic::{Request, Response, Status};

use crate::error;
use crate::support::{self, boolean, integer, RpcStream};
use crate::wire::sila2::org::silastandard::test::observablepropertytest::v1::observable_property_test_server::ObservablePropertyTest;
use crate::wire::sila2::org::silastandard::test::observablepropertytest::v1::{
    SetValueParameters, SetValueResponses, SubscribeAlternatingParameters, SubscribeAlternatingResponses,
    SubscribeEditableParameters, SubscribeEditableResponses, SubscribeFixedValueParameters, SubscribeFixedValueResponses,
};

pub struct Feature {
    editable: Arc<Editable>,
}

struct Editable {
    value: Mutex<i64>,
    generation: Mutex<u64>,
    notify: Notify,
}

impl Default for Feature {
    fn default() -> Self {
        Self {
            editable: Arc::new(Editable {
                value: Mutex::new(0),
                generation: Mutex::new(0),
                notify: Notify::new(),
            }),
        }
    }
}

#[tonic::async_trait]
impl ObservablePropertyTest for Feature {
    type Subscribe_FixedValueStream = RpcStream<SubscribeFixedValueResponses>;
    type Subscribe_AlternatingStream = RpcStream<SubscribeAlternatingResponses>;
    type Subscribe_EditableStream = RpcStream<SubscribeEditableResponses>;

    async fn set_value(
        &self,
        request: Request<SetValueParameters>,
    ) -> Result<Response<SetValueResponses>, Status> {
        let Some(value) = request.into_inner().value else {
            return Err(error::validation(
                "org.silastandard/test/ObservablePropertyTest/v1/Command/SetValue/Parameter/Value",
                "Missing parameter: 'Value'",
            ));
        };
        *support::guard(&self.editable.value) = value.value;
        *support::guard(&self.editable.generation) += 1;
        self.editable.notify.notify_waiters();
        Ok(Response::new(SetValueResponses {}))
    }

    async fn subscribe_fixed_value(
        &self,
        _request: Request<SubscribeFixedValueParameters>,
    ) -> Result<Response<Self::Subscribe_FixedValueStream>, Status> {
        Ok(Response::new(Box::pin(stream::iter([Ok(
            SubscribeFixedValueResponses {
                fixed_value: Some(integer(42)),
            },
        )]))))
    }

    async fn subscribe_alternating(
        &self,
        _request: Request<SubscribeAlternatingParameters>,
    ) -> Result<Response<Self::Subscribe_AlternatingStream>, Status> {
        Ok(Response::new(Box::pin(stream::unfold(
            (true, false),
            |(value, delay)| async move {
                if delay {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
                let item = Ok(SubscribeAlternatingResponses {
                    alternating: Some(boolean(value)),
                });
                Some((item, (!value, true)))
            },
        ))))
    }

    async fn subscribe_editable(
        &self,
        _request: Request<SubscribeEditableParameters>,
    ) -> Result<Response<Self::Subscribe_EditableStream>, Status> {
        let editable = Arc::clone(&self.editable);
        Ok(Response::new(Box::pin(stream::unfold(None, move |seen| {
            let editable = Arc::clone(&editable);
            async move {
                next_editable(&editable, seen)
                    .await
                    .map(|generation| (Ok(editable_response(generation.0)), Some(generation.1)))
            }
        }))))
    }
}

async fn next_editable(editable: &Editable, seen: Option<u64>) -> Option<(i64, u64)> {
    loop {
        let pending = editable.notify.notified();
        let value = *support::guard(&editable.value);
        let generation = *support::guard(&editable.generation);
        if seen != Some(generation) {
            return Some((value, generation));
        }
        pending.await;
    }
}

fn editable_response(value: i64) -> SubscribeEditableResponses {
    SubscribeEditableResponses {
        editable: Some(integer(value)),
    }
}
