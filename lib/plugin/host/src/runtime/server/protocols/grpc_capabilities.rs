use az_plugin_contract::{
    EnhancementRequest,
    grpc::{
        CapabilityDescriptor, CapabilityEffect as Effect, CapabilityInvokeResponse,
        CapabilityListResponse, DescribeRequest, InvokeRequest, ListRequest,
        capabilities_server::Capabilities,
    },
};
use tonic::{Request, Response, Status};

use super::{
    super::{RuntimeState, capabilities::CapabilityEffect},
    access,
    grpc::{Deadline, status},
};

pub(super) struct GrpcCapabilities(pub RuntimeState);

#[tonic::async_trait]
impl Capabilities for GrpcCapabilities {
    async fn list(
        &self,
        request: Request<ListRequest>,
    ) -> Result<Response<CapabilityListResponse>, Status> {
        let headers = request.metadata().clone().into_headers();
        let session = access::session(&self.0, &headers).await.map_err(status)?;
        let invocation = access::capability_invocation(&self.0, &session, None).map_err(status)?;
        let paths = invocation
            .catalog()
            .map_err(|error| status(error.into()))?
            .map(|contract| contract.path.clone())
            .collect();
        Ok(Response::new(CapabilityListResponse { paths }))
    }

    async fn describe(
        &self,
        request: Request<DescribeRequest>,
    ) -> Result<Response<CapabilityDescriptor>, Status> {
        let headers = request.metadata().clone().into_headers();
        let session = access::session(&self.0, &headers).await.map_err(status)?;
        let path = request.into_inner().path;
        az_plugin_contract::validate_enhancement_path(&path)
            .map_err(|_| Status::invalid_argument("能力路径无效"))?;
        let invocation = access::capability_invocation(&self.0, &session, None).map_err(status)?;
        let contract = invocation
            .catalog()
            .map_err(|error| status(error.into()))?
            .find(|contract| contract.path == path)
            .ok_or_else(|| Status::not_found("能力不可见或不存在"))?;
        let effect = match contract.effect {
            CapabilityEffect::Read => Effect::Read,
            CapabilityEffect::IdempotentWrite => Effect::IdempotentWrite,
            CapabilityEffect::Write => Effect::Write,
        };
        let input_schema_json = serde_json::to_vec(&contract.input_schema)
            .map_err(|_| Status::internal("编码能力输入契约失败"))?;
        let output_schema_json = serde_json::to_vec(&contract.output_schema)
            .map_err(|_| Status::internal("编码能力输出契约失败"))?;
        Ok(Response::new(CapabilityDescriptor {
            path,
            description: contract.description.clone(),
            effect: effect.into(),
            input_schema_json,
            output_schema_json,
        }))
    }

    async fn invoke(
        &self,
        request: Request<InvokeRequest>,
    ) -> Result<Response<CapabilityInvokeResponse>, Status> {
        let deadline = request.extensions().get::<Deadline>().map(|value| value.0);
        let headers = request.metadata().clone().into_headers();
        let session = access::session(&self.0, &headers).await.map_err(status)?;
        let request = request.into_inner();
        let input = serde_json::from_slice(&request.input_json)
            .map_err(|_| Status::invalid_argument("input_json 必须是完整 JSON 值"))?;
        let result = access::invoke_capability(
            &self.0,
            &session,
            EnhancementRequest {
                path: request.path,
                input,
            },
            deadline,
        )
        .await;
        let output = super::grpc::invocation_result(result, deadline)?;
        let output_json =
            serde_json::to_vec(&output).map_err(|_| Status::internal("编码能力结果失败"))?;
        Ok(Response::new(CapabilityInvokeResponse { output_json }))
    }
}
