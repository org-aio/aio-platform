use std::sync::{Arc, atomic::Ordering};

use anyhow::{Context, Result};
use az_plugin_contract::{
    CapabilityGrants, EnhancementRequest, InvocationScope, RequestContext,
    grpc::{InvokeRequest, ListRequest, enhancements_client::EnhancementsClient},
};
use az_plugin_enhancement::Package;
use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};

use super::support::*;
use crate::{
    generated::worker::{WorkerService, model::PairRequest},
    identity::SessionContext,
    runtime::server::{
        enhancements::{CapabilityEnvironment, Enhancements},
        plugins::*,
    },
};

const ROOT: &str = "/v1/workflow/root";
const CHILD: &str = "/v1/workflow/worker";

fn package(reverse: bool) -> Result<Package> {
    let root = if reverse {
        (
            "typescript",
            "export function invoke(input, context) { context.tenant_id = 'forged'; return call_enhancement('/v1/workflow/worker', input); }",
        )
    } else {
        (
            "rhai",
            "fn invoke(input, context) { context.tenant_id = \"forged\"; call_enhancement(\"/v1/workflow/worker\", input) }",
        )
    };
    let child = if reverse {
        (
            "rhai",
            "fn invoke(input, context) { let route = if input.action == \"list\" { \"list\" } else { \"tasks/\" + input.action }; call_capability(\"/v1/workers/\" + route, input.value) }",
        )
    } else {
        (
            "typescript",
            "export function invoke(input) { const route = input.action === 'list' ? 'list' : 'tasks/' + input.action; return call_capability('/v1/workers/' + route, input.value); }",
        )
    };
    let submission = json!({"id":"00000000-0000-4000-8000-000000000001","worker_id":"validation-device","capability":"desktop.open-app","input":{"application":"Calculator"}});
    let examples = json!([
        {"input":{"action":"list","value":{}},"output":[],"capabilities":[{"path":"/v1/workers/list","input":{},"output":[]}]},
        {"input":{"action":"submit","value":submission},"output":{"id":"sample-task"},"capabilities":[{"path":"/v1/workers/tasks/submit","input":submission,"output":{"id":"sample-task"}}]}
    ]);
    Ok(serde_json::from_value(
        json!({"schema_version":1,"enhancements":[
            {"path":ROOT,"language":root.0,"source":root.1,"calls":[CHILD],"examples":examples},
            {"path":CHILD,"language":child.0,"source":child.1,"capabilities":["/v1/workers/list","/v1/workers/tasks/get","/v1/workers/tasks/submit","/v1/workers/tasks/cancel"],"examples":examples}
        ]}),
    )?)
}

fn input(action: &str, value: Value) -> Value {
    json!({"action":action,"value":value})
}

fn tool(input: Value) -> CallToolRequestParams {
    CallToolRequestParams::new("v1.workflow.root")
        .with_arguments(serde_json::Map::from_iter([("input".into(), input)]))
}

fn rpc(input: Value) -> Result<InvokeRequest> {
    Ok(InvokeRequest {
        path: ROOT.into(),
        input_json: serde_json::to_vec(&input)?,
    })
}

fn session(fixture: &Fixture) -> SessionContext {
    SessionContext {
        session_id: "capable".into(),
        user_id: "capable".into(),
        tenant_id: fixture.identity.tenant.clone(),
        account: "capable".into(),
        display_name: String::new(),
        tenant_label: String::new(),
        permissions: vec![],
    }
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn omitting_capability_plugin_removes_discovery_and_prevents_activation() -> Result<()> {
    let fixture = Fixture::open().await?;
    let outcome = async {
        assert!(
            fixture
                .state
                .service::<RuntimeCapabilities>()?
                .catalog(&fixture.identity.tenant)?
                .is_empty()
        );
        assert!(fixture.publish(package(false)?, 0).await.is_err());
        assert!(
            fixture
                .state
                .service::<Enhancements>()?
                .head(&fixture.identity.tenant)
                .await?
                .revision
                .is_none()
        );
        Ok::<_, anyhow::Error>(())
    }
    .await;
    let shutdown = fixture.state.shutdown().await;
    fixture.server.abort();
    outcome?;
    shutdown
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn scripts_use_real_worker_services_through_mcp_grpc_upgrade_and_revocation() -> Result<()> {
    let fixture = Fixture::with_plugins(vec![
        Arc::new(WorkerCapabilitiesPlugin),
        Arc::new(WorkerServicePlugin),
        Arc::new(StorePlugin),
    ])
    .await?;
    let outcome =
        tokio::time::timeout(std::time::Duration::from_secs(60), exercise(&fixture)).await;
    let shutdown = fixture.state.shutdown().await;
    fixture.server.abort();
    outcome??;
    shutdown
}

async fn exercise(fixture: &Fixture) -> Result<()> {
    let client = reqwest::Client::builder().no_proxy().build()?;
    let catalog_url = format!("{}/api/runtime/capabilities", fixture.base);
    assert_eq!(client.get(&catalog_url).send().await?.status(), 401);
    let catalog: Value = client
        .get(&catalog_url)
        .bearer_auth("manager")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let descriptors = catalog.as_array().context("能力目录不是数组")?;
    assert_eq!(descriptors.len(), 4);
    let submit = descriptors
        .iter()
        .find(|entry| entry["path"] == "/v1/workers/tasks/submit")
        .context("缺少任务能力")?;
    assert_eq!(submit["effect"], "idempotent_write");
    assert_eq!(submit["input_schema"]["additionalProperties"], false);
    assert!(submit["output_schema"]["properties"].get("id").is_some());

    let workers = fixture.state.service::<dyn WorkerService>()?;
    let owner = session(fixture);
    let paired = workers
        .pair(PairRequest {
            label: "Script device".into(),
            platform: "test".into(),
            capabilities: vec!["desktop.open-app".into()],
            machine_id: Some(uuid::Uuid::new_v4().to_string()),
        })
        .await?;
    workers.approve(&owner, &paired.code).await?;
    let first = fixture.publish(package(false)?, 0).await?;
    // 发布样例含提交任务的分支，但只能重放样例，不能向真实设备插入任务。
    assert!(workers.tasks(&owner).await?.is_empty());

    let mcp = mcp_client(&fixture.base, "capable").await?;
    let manager = mcp_client(&fixture.base, "manager").await?;
    let mut grpc = EnhancementsClient::connect(fixture.base.clone()).await?;
    let tools = mcp.list_all_tools().await?;
    assert_eq!(tools.len(), 6);
    assert_eq!(
        tools
            .iter()
            .filter(|tool| tool.name.starts_with("runtime."))
            .count(),
        4
    );
    assert!(manager.list_all_tools().await?.is_empty());
    assert_eq!(
        manager
            .call_tool(tool(input("list", json!({}))))
            .await?
            .is_error,
        Some(true)
    );
    let list = mcp.call_tool(tool(input("list", json!({})))).await?;
    let list = list.structured_content.context("缺少设备列表")?;
    assert_eq!(list["revision"], first);
    assert_eq!(list["output"][0]["id"], paired.device_id);
    assert_eq!(
        list["output"].as_array().context("设备列表类型错误")?.len(),
        1
    );

    let id = uuid::Uuid::new_v4().to_string();
    let task = json!({"id":id,"worker_id":paired.device_id,"capability":"desktop.open-app","input":{"application":"Calculator"}});
    let submitted = mcp.call_tool(tool(input("submit", task.clone()))).await?;
    assert_eq!(
        submitted.structured_content.context("缺少任务结果")?["output"]["id"],
        id
    );
    let retried = grpc
        .invoke(auth(rpc(input("submit", task.clone()))?, "capable")?)
        .await?
        .into_inner();
    assert_eq!(
        serde_json::from_slice::<Value>(&retried.output_json)?["id"],
        id
    );
    assert_eq!(workers.tasks(&owner).await?.len(), 1);
    let mut conflict = task.clone();
    conflict["input"] = json!({"application":"Other"});
    assert!(
        grpc.invoke(auth(rpc(input("submit", conflict))?, "capable")?)
            .await
            .is_err()
    );
    let mut forged = task;
    forged["user_id"] = json!("another-user");
    assert!(
        grpc.invoke(auth(rpc(input("submit", forged))?, "capable")?)
            .await
            .is_err()
    );
    let fetched = grpc
        .invoke(auth(rpc(input("get", json!({"id":id})))?, "capable")?)
        .await?
        .into_inner();
    assert_eq!(
        serde_json::from_slice::<Value>(&fetched.output_json)?["state"],
        "queued"
    );

    let second = fixture.publish(package(true)?, 1).await?;
    for _ in 0..2 {
        let cancelled = mcp
            .call_tool(tool(input("cancel", json!({"id":id}))))
            .await?;
        let result = cancelled.structured_content.context("缺少取消结果")?;
        assert_eq!(result["revision"], second);
        assert_eq!(result["output"]["state"], "cancelled");
    }
    assert_eq!(workers.tasks(&owner).await?.len(), 1);
    let service = fixture.state.service::<Enhancements>()?;
    service
        .activate(
            &owner.tenant_id,
            "manager",
            crate::runtime::server::enhancements::Activation {
                revision: Some(first.clone()),
                expected_generation: 2,
            },
            true,
            async { Ok(()) },
        )
        .await?
        .context("回滚失败")?;
    let result = mcp.call_tool(tool(input("list", json!({})))).await?;
    assert_eq!(
        result.structured_content.context("缺少回滚结果")?["revision"],
        first
    );

    // 使用全新增强服务恢复数据库版本，宿主能力仍绑定到同一个真实服务容器。
    let restored = Enhancements::open(
        fixture.state.service::<sqlx::PgPool>()?.as_ref().clone(),
        az_plugin_enhancement::default_runtime()?,
        Some(CapabilityEnvironment {
            services: Arc::downgrade(&fixture.state.extensions),
            identity: fixture.identity.clone(),
        }),
    )
    .await?;
    let mut scope = InvocationScope {
        source_id: "test-plugin".into(),
        revision: "installed-package".into(),
        context: RequestContext {
            tenant_id: Some(owner.tenant_id.clone()),
            user_id: Some(owner.user_id.clone()),
            session_id: Some(owner.session_id.clone()),
            request_id: uuid::Uuid::new_v4().to_string(),
        },
        grants: CapabilityGrants {
            enhancements: vec![ROOT.into(), CHILD.into()],
            runtime: fixture
                .state
                .service::<RuntimeCapabilities>()?
                .catalog(&fixture.identity.tenant)?
                .into_iter()
                .map(|entry| entry.path)
                .collect(),
            ..Default::default()
        },
    };
    let request = EnhancementRequest {
        path: ROOT.into(),
        input: input("get", json!({"id":id})),
    };
    assert_eq!(
        restored.invoke(&scope, request.clone(), None).await?.output["state"],
        "cancelled"
    );
    scope.context.tenant_id = Some("other-tenant".into());
    assert!(restored.invoke(&scope, request, None).await.is_err());

    fixture
        .identity
        .runtime_allowed
        .store(false, Ordering::SeqCst);
    assert!(mcp.list_all_tools().await?.is_empty());
    assert!(
        grpc.list(auth(ListRequest {}, "capable")?)
            .await?
            .into_inner()
            .paths
            .is_empty()
    );
    assert!(
        grpc.invoke(auth(rpc(input("list", json!({})))?, "capable")?)
            .await
            .is_err()
    );
    fixture
        .identity
        .runtime_allowed
        .store(true, Ordering::SeqCst);
    fixture.identity.revoked.store(true, Ordering::SeqCst);
    assert_eq!(
        grpc.invoke(auth(rpc(input("list", json!({})))?, "capable")?)
            .await
            .err()
            .context("撤销会话仍可执行")?
            .code(),
        tonic::Code::Unauthenticated
    );
    mcp.cancel().await?;
    manager.cancel().await?;
    Ok(())
}

#[derive(Clone)]
struct WaitingProvider {
    entered: tokio::sync::mpsc::UnboundedSender<()>,
    dropped: Arc<std::sync::atomic::AtomicUsize>,
}

struct WaitingPlugin(WaitingProvider);

impl az_plugin_core::Plugin<RuntimeAssembly> for WaitingPlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> az_plugin_core::PluginFuture<'a> {
        Box::pin(async move {
            assembly
                .services
                .add_value(self.0.clone())
                .bind::<dyn CapabilityProvider, WaitingProvider>();
            Ok(())
        })
    }
}

struct PendingCall(Arc<std::sync::atomic::AtomicUsize>);
impl Drop for PendingCall {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl CapabilityProvider for WaitingProvider {
    fn capabilities(&self) -> Vec<RuntimeCapability> {
        vec![RuntimeCapability {
            path: "/v1/test/wait".into(),
            description: "测试异步等待".into(),
            effect: CapabilityEffect::Read,
            input_schema: schemars::schema_for!(Value),
            output_schema: schemars::schema_for!(Value),
        }]
    }
    async fn invoke(&self, _: &CapabilityRequest) -> Result<Value> {
        let _pending = PendingCall(self.dropped.clone());
        self.entered.send(())?;
        std::future::pending().await
    }
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn cancelling_host_capabilities_drops_async_io_and_releases_script_slots() -> Result<()> {
    let (entered, mut entries) = tokio::sync::mpsc::unbounded_channel();
    let dropped = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let fixture = Fixture::with_plugins(vec![Arc::new(WaitingPlugin(WaitingProvider {
        entered,
        dropped: dropped.clone(),
    }))])
    .await?;
    let outcome = async {
        let candidate = serde_json::from_value(json!({"schema_version":1,"enhancements":[{
            "path":"/v1/test/waiter","language":"typescript","source":"export function invoke(input) { return input ? call_capability('/v1/test/wait', input) : 0; }",
            "capabilities":["/v1/test/wait"],"examples":[{"input":true,"output":0,"capabilities":[{"path":"/v1/test/wait","input":true,"output":0}]}]
        }]}))?;
        fixture.publish(candidate, 0).await?;
        assert_eq!(dropped.load(Ordering::SeqCst), 0);
        let service = fixture.state.service::<Enhancements>()?;
        let scope = InvocationScope { source_id: "test".into(), revision: "test".into(), context: RequestContext { tenant_id: Some(fixture.identity.tenant.clone()), ..Default::default() }, grants: CapabilityGrants { enhancements: vec!["/v1/test/waiter".into()], runtime: vec!["/v1/test/wait".into()], ..Default::default() } };
        let mut tasks = Vec::new();
        for _ in 0..4 {
            let service = service.clone();
            let scope = scope.clone();
            tasks.push(tokio::spawn(async move { service.invoke(&scope, EnhancementRequest { path: "/v1/test/waiter".into(), input: json!(true) }, None).await }));
        }
        for _ in 0..4 {
            tokio::time::timeout(std::time::Duration::from_secs(2), entries.recv()).await?.context("能力调用未启动")?;
        }
        for task in tasks {
            task.abort();
            assert!(task.await.err().is_some_and(|error| error.is_cancelled()));
        }
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while dropped.load(Ordering::SeqCst) < 4 { tokio::task::yield_now().await; }
            loop {
                match service.invoke(&scope, EnhancementRequest { path: "/v1/test/waiter".into(), input: json!(false) }, None).await {
                    Ok(response) => { assert_eq!(response.output, 0); break; }
                    Err(error) if error.to_string().contains("并发已满") => tokio::task::yield_now().await,
                    Err(error) => return Err(error),
                }
            }
            Ok::<_, anyhow::Error>(())
        }).await??;
        Ok::<_, anyhow::Error>(())
    }.await;
    let shutdown = fixture.state.shutdown().await;
    fixture.server.abort();
    outcome?;
    shutdown
}
