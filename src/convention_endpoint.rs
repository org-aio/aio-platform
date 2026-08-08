use std::{collections::BTreeMap, fmt::Debug, future::Future, pin::Pin, sync::Arc};

use anyhow::{Context, Result, bail, ensure};
use axum::{
    Json, Router,
    body::to_bytes,
    extract::{RawPathParams, Request, State},
    http::StatusCode,
    routing::{MethodFilter, on},
};
use az_plugin_core::http::{ApiError, ApiResponse, ok_json};
use rudi::Context as RudiContext;
use serde_json::Value;

use crate::{PageEndpointSource, ProgramImage, RestMethod, SymbolId};

const MAX_CONVENTION_BODY_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConventionEndpointRequest {
    pub path: BTreeMap<String, String>,
    pub query: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub body: Value,
}

pub type ConventionEndpointFuture<'a> = Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>>;

pub trait ConventionEndpointProvider: Send + Sync + Debug {
    fn key(&self) -> &'static str;

    fn endpoint_id(&self) -> &'static str;

    fn handle(&self, request: ConventionEndpointRequest) -> ConventionEndpointFuture<'_>;
}

pub type DynConventionEndpointProvider = Arc<dyn ConventionEndpointProvider>;

#[derive(Clone, Debug, Default)]
pub struct ConventionEndpointIndex {
    providers: BTreeMap<String, DynConventionEndpointProvider>,
}

impl ConventionEndpointIndex {
    pub fn from_context(context: &mut RudiContext) -> Result<Self> {
        let provider_names = context
            .get_providers_by_type::<DynConventionEndpointProvider>()
            .into_iter()
            .map(|provider| provider.definition().key.name.to_string())
            .collect::<Vec<_>>();
        let mut providers = BTreeMap::new();
        for provider_name in provider_names {
            let provider = context
                .resolve_option_with_name::<DynConventionEndpointProvider>(provider_name.clone())
                .with_context(|| format!("无法解析约定接口 Provider: {provider_name}"))?;
            ensure!(
                provider.key() == provider_name,
                "约定接口的 Rudi name 与 Provider key 不一致: {provider_name} != {}",
                provider.key()
            );
            SymbolId::parse(provider.endpoint_id()).with_context(|| {
                format!("约定接口 Provider {} 的 endpoint_id 无效", provider.key())
            })?;
            if providers
                .insert(provider.endpoint_id().to_owned(), provider)
                .is_some()
            {
                bail!("约定接口 Provider endpoint_id 重复: {provider_name}");
            }
        }
        Ok(Self { providers })
    }

    pub fn router(&self, image: &ProgramImage) -> Result<Router> {
        let mut router = Router::new();
        let mut routes = BTreeMap::new();
        for page in image.pages.values() {
            for endpoint in &page.endpoints {
                if endpoint.source != PageEndpointSource::Convention {
                    continue;
                }
                let route_key = (endpoint.method, endpoint.path.clone());
                if let Some(existing) = routes.insert(route_key.clone(), endpoint.id.clone()) {
                    bail!(
                        "约定接口路由重复: {} {} ({existing}, {})",
                        endpoint.method.as_str(),
                        endpoint.path,
                        endpoint.id
                    );
                }
                let state = ConventionEndpointRoute {
                    endpoint_id: endpoint.id.clone(),
                    provider: self.providers.get(&endpoint.id).cloned(),
                };
                let method_router = on(method_filter(endpoint.method), dispatch).with_state(state);
                router = router.route(&endpoint.path, method_router);
            }
        }
        Ok(router)
    }
}

#[derive(Clone)]
struct ConventionEndpointRoute {
    endpoint_id: String,
    provider: Option<DynConventionEndpointProvider>,
}

async fn dispatch(
    State(route): State<ConventionEndpointRoute>,
    path: RawPathParams,
    request: Request,
) -> Result<Json<ApiResponse<Value>>, ApiError> {
    let provider = route.provider.ok_or_else(|| {
        ApiError::new(
            StatusCode::NOT_IMPLEMENTED,
            format!("约定接口尚未编译后端 Provider: {}", route.endpoint_id),
        )
    })?;
    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, MAX_CONVENTION_BODY_BYTES)
        .await
        .map_err(|error| ApiError::bad_request(format!("读取约定接口请求体失败: {error}")))?;
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .map_err(|error| ApiError::bad_request(format!("约定接口请求体不是 JSON: {error}")))?
    };
    let request = ConventionEndpointRequest {
        path: path
            .iter()
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect(),
        query: parts.uri.query().map(ToOwned::to_owned),
        headers: parts
            .headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.to_string(), value.to_owned()))
            })
            .collect(),
        body,
    };
    provider
        .handle(request)
        .await
        .map(ok_json)
        .map_err(ApiError::from)
}

const fn method_filter(method: RestMethod) -> MethodFilter {
    match method {
        RestMethod::Get => MethodFilter::GET,
        RestMethod::Post => MethodFilter::POST,
        RestMethod::Put => MethodFilter::PUT,
        RestMethod::Patch => MethodFilter::PATCH,
        RestMethod::Delete => MethodFilter::DELETE,
    }
}

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::Request};
    use serde_json::json;
    use tower::ServiceExt;

    use crate::{
        CapabilityCatalog, DefinitionState, EndpointImplementationDefinition, ImageTarget,
        PageDefinition, PageEndpointDefinition, PageRendererDefinition, ProgramCompiler,
        ProgramDefinition, RouteDefinition,
    };

    use super::*;

    const ENDPOINT_ID: &str = "5cbf910c-05af-4537-94d3-673c3b4c444b";

    #[derive(Debug)]
    struct TestEndpoint;

    impl ConventionEndpointProvider for TestEndpoint {
        fn key(&self) -> &'static str {
            "test::orders::submit"
        }

        fn endpoint_id(&self) -> &'static str {
            ENDPOINT_ID
        }

        fn handle(&self, request: ConventionEndpointRequest) -> ConventionEndpointFuture<'_> {
            Box::pin(async move {
                ensure!(
                    request.path.get("order_id").map(String::as_str) == Some("order-7"),
                    "路径参数未进入约定接口请求"
                );
                ensure!(
                    request.query.as_deref() == Some("dry_run=true"),
                    "查询参数缺失"
                );
                ensure!(request.body["quantity"] == 2, "JSON 请求体缺失");
                Ok(json!({ "accepted": true }))
            })
        }
    }

    fn image() -> Result<ProgramImage> {
        let mut definition = ProgramDefinition::empty("orders", "订单");
        let page_id = SymbolId::new();
        definition.pages.push(PageDefinition {
            id: page_id,
            name: "orders".to_owned(),
            title: "订单".to_owned(),
            state: DefinitionState::Known,
            renderer: PageRendererDefinition::ConventionFile,
            endpoints: vec![PageEndpointDefinition {
                id: SymbolId::parse(ENDPOINT_ID)?,
                title: "提交订单".to_owned(),
                description: "提交一笔订单".to_owned(),
                state: DefinitionState::Known,
                implementation: EndpointImplementationDefinition::Convention,
                method: RestMethod::Post,
                path: "/api/orders/{order_id}".to_owned(),
                inputs: Vec::new(),
                outputs: Vec::new(),
            }],
        });
        definition.routes.push(RouteDefinition {
            id: SymbolId::new(),
            name: "orders".to_owned(),
            path: "/orders".to_owned(),
            page_id,
            state: DefinitionState::Known,
            required_permissions: Vec::new(),
        });
        ProgramCompiler::new("test", &CapabilityCatalog::default())
            .compile(&definition, "revision", ImageTarget::Universal)
            .map_err(|failure| anyhow::anyhow!(failure.to_string()))
    }

    #[tokio::test]
    async fn dispatches_compiled_convention_endpoint_to_provider() -> Result<()> {
        let index = ConventionEndpointIndex {
            providers: BTreeMap::from([(
                ENDPOINT_ID.to_owned(),
                Arc::new(TestEndpoint) as DynConventionEndpointProvider,
            )]),
        };
        let router = index.router(&image()?)?;
        let request = Request::builder()
            .method("POST")
            .uri("/api/orders/order-7?dry_run=true")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"quantity":2}"#))?;

        let response = router.oneshot(request).await?;

        assert_eq!(response.status(), StatusCode::OK);
        Ok(())
    }

    #[tokio::test]
    async fn returns_not_implemented_before_provider_is_compiled() -> Result<()> {
        let router = ConventionEndpointIndex::default().router(&image()?)?;
        let request = Request::builder()
            .method("POST")
            .uri("/api/orders/order-7")
            .body(Body::empty())?;

        let response = router.oneshot(request).await?;

        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
        Ok(())
    }
}
