use az_plugin_contract::{
    EnhancementRequest,
    grpc::{
        DescribeRequest, DescribeResponse, InvokeRequest, InvokeResponse, ListRequest,
        ListResponse, enhancements_server::Enhancements,
    },
};
use tonic::{Request, Response, Status};

use super::super::RuntimeState;
use super::{
    access,
    grpc::{Deadline, status},
};

pub(super) struct GrpcEnhancements(pub RuntimeState);

#[tonic::async_trait]
impl Enhancements for GrpcEnhancements {
    async fn list(&self, request: Request<ListRequest>) -> Result<Response<ListResponse>, Status> {
        let headers = request.metadata().clone().into_headers();
        let session = access::session(&self.0, &headers).await.map_err(status)?;
        let (revision, contracts) = access::catalog(&self.0, &session).await.map_err(status)?;
        let paths = contracts
            .into_iter()
            .map(|contract| contract.path)
            .collect();
        Ok(Response::new(ListResponse { revision, paths }))
    }

    async fn describe(
        &self,
        request: Request<DescribeRequest>,
    ) -> Result<Response<DescribeResponse>, Status> {
        let headers = request.metadata().clone().into_headers();
        let session = access::session(&self.0, &headers).await.map_err(status)?;
        let path = request.into_inner().path;
        az_plugin_contract::validate_enhancement_path(&path)
            .map_err(|_| Status::invalid_argument("增强路径无效"))?;
        let (revision, contracts) = access::catalog(&self.0, &session).await.map_err(status)?;
        let contract = contracts
            .into_iter()
            .find(|contract| contract.path == path)
            .ok_or_else(|| Status::not_found("增强能力不可见或不存在"))?;
        let revision = revision.ok_or_else(|| Status::not_found("增强未激活"))?;
        let input_schema_json = serde_json::to_vec(&contract.input_schema)
            .map_err(|_| Status::internal("编码增强输入契约失败"))?;
        let output_schema_json = serde_json::to_vec(&contract.output_schema)
            .map_err(|_| Status::internal("编码增强输出契约失败"))?;
        Ok(Response::new(DescribeResponse {
            revision,
            path,
            input_schema_json,
            output_schema_json,
        }))
    }

    async fn invoke(
        &self,
        request: Request<InvokeRequest>,
    ) -> Result<Response<InvokeResponse>, Status> {
        let deadline = request.extensions().get::<Deadline>().map(|value| value.0);
        let headers = request.metadata().clone().into_headers();
        let session = access::session(&self.0, &headers).await.map_err(status)?;
        let request = request.into_inner();
        let input = serde_json::from_slice(&request.input_json)
            .map_err(|_| Status::invalid_argument("input_json 必须是完整 JSON 值"))?;
        let result = access::invoke(
            &self.0,
            &session,
            EnhancementRequest {
                path: request.path,
                input,
            },
            deadline,
        )
        .await;
        let response = super::grpc::invocation_result(result, deadline)?;
        let output_json = serde_json::to_vec(&response.output)
            .map_err(|_| Status::internal("编码增强结果失败"))?;
        Ok(Response::new(InvokeResponse {
            revision: response.revision,
            output_json,
        }))
    }
}
