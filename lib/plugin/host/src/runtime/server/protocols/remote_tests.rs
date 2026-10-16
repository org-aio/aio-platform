use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use az_plugin_contract::grpc::{InvokeRequest, enhancements_client::EnhancementsClient};
use az_plugin_enhancement::Package;
use serde_json::{Value, json};

use super::support::*;
use crate::runtime::server::plugins::{RemoteCapabilitiesPlugin, RuntimeCapabilities};

#[path = "remote_fixture.rs"]
mod upstream;
use upstream::Upstream;

#[path = "remote_reload_tests.rs"]
mod reload;

fn package(protocol: &str, typescript: bool) -> Result<Package> {
    let path = format!("/v1/remote/{protocol}");
    let source = if typescript {
        format!("export function invoke(input) {{ return call_capability('{path}', input); }}")
    } else {
        format!("fn invoke(input, context) {{ call_capability(\"{path}\", input) }}")
    };
    Ok(serde_json::from_value(
        json!({"schema_version":1,"enhancements":[{
            "path":"/v1/remote/double","language":if typescript {"typescript"} else {"rhai"},"source":source,"capabilities":[path],
            "examples":[{"input":{"value":3},"output":{"value":6},"capabilities":[{"path":path,"input":{"value":3},"output":{"value":6}}]}]
        }]}),
    )?)
}

fn invoke(value: Value) -> Result<InvokeRequest> {
    Ok(InvokeRequest {
        path: "/v1/remote/double".into(),
        input_json: serde_json::to_vec(&value)?,
    })
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL 和 AIO_REMOTE_TEST_CREDENTIAL=remote-test"]
async fn imports_external_protocols_with_tenant_grants_contracts_and_cancellation() -> Result<()> {
    ensure!(
        std::env::var("AIO_REMOTE_TEST_CREDENTIAL")? == "remote-test",
        "只使用测试凭据"
    );
    let remote = Upstream::open().await?;
    let fixture = Fixture::configured(
        vec![Arc::new(RemoteCapabilitiesPlugin)],
        Some(remote.imports()),
    )
    .await?;
    let result = tokio::time::timeout(Duration::from_secs(60), exercise(&fixture, &remote)).await;
    let cleanup = fixture.state.shutdown().await;
    fixture.server.abort();
    result??;
    cleanup?;
    assert!(
        fixture
            .state
            .service::<RuntimeCapabilities>()?
            .catalog(&fixture.identity.tenant)?
            .is_empty()
    );
    Ok(())
}

async fn exercise(fixture: &Fixture, remote: &Upstream) -> Result<()> {
    let registry = fixture.state.service::<RuntimeCapabilities>()?;
    assert_eq!(registry.catalog(&fixture.identity.tenant)?.len(), 2);
    assert!(
        registry
            .catalog("other")
            .is_ok_and(|items| items.is_empty())
    );
    let mcp = mcp_client(&fixture.base, "remote").await?;
    let manager = mcp_client(&fixture.base, "manager").await?;
    let other = mcp_client(&fixture.base, "other").await?;
    let mut grpc = EnhancementsClient::connect(fixture.base.clone()).await?;
    let mut generation = 0;
    for protocol in ["mcp", "grpc"] {
        for typescript in [false, true] {
            // 切换协议改变调用记录，先停用再激活独立契约；同一协议跨语言升级沿用旧样例。
            if protocol == "grpc" && !typescript {
                fixture
                    .state
                    .service::<crate::runtime::server::enhancements::Enhancements>()?
                    .activate(&fixture.identity.tenant, "test", None, generation, false)
                    .await?
                    .context("停用失败")?;
                generation += 1;
            }
            let before = remote.counts.calls.load(Ordering::SeqCst);
            let revision = fixture
                .publish(package(protocol, typescript)?, generation)
                .await?;
            generation += 1;
            assert_eq!(
                remote.counts.calls.load(Ordering::SeqCst),
                before,
                "候选验证不能调用真实远端"
            );
            assert_eq!(mcp.list_all_tools().await?.len(), 1);
            assert!(manager.list_all_tools().await?.is_empty());
            assert!(other.list_all_tools().await?.is_empty());
            let response = grpc
                .invoke(auth(invoke(json!({"value":21}))?, "remote")?)
                .await?
                .into_inner();
            assert_eq!(response.revision, revision);
            assert_eq!(
                serde_json::from_slice::<Value>(&response.output_json)?,
                json!({"value":42})
            );
            if protocol == "grpc" {
                let response = grpc
                    .invoke(auth(invoke(json!({"value":0}))?, "remote")?)
                    .await?
                    .into_inner();
                assert_eq!(
                    serde_json::from_slice::<Value>(&response.output_json)?,
                    json!({"value":0})
                );
                let mut request = auth(invoke(json!({"value":2}))?, "remote")?;
                request.set_timeout(Duration::from_secs(2));
                grpc.invoke(request).await?;
                let forwarded = remote.counts.last_timeout_micros.load(Ordering::SeqCst);
                assert!(
                    (1..=2_000_000).contains(&forwarded),
                    "入口截止时间没有传到上游: {forwarded}"
                );
            }
            let response = mcp
                .call_tool(
                    rmcp::model::CallToolRequestParams::new("v1.remote.double").with_arguments(
                        json!({"input":{"value":7}})
                            .as_object()
                            .cloned()
                            .context("参数不是对象")?,
                    ),
                )
                .await?;
            assert_eq!(
                response.structured_content.context("缺少结果")?["output"],
                json!({"value":14})
            );
            for value in [
                json!({"value":"bad"}),
                json!({"value":1,"tenant_id":"forged"}),
            ] {
                let before = remote.counts.calls.load(Ordering::SeqCst);
                assert!(grpc.invoke(auth(invoke(value)?, "remote")?).await.is_err());
                assert_eq!(remote.counts.calls.load(Ordering::SeqCst), before);
            }
            assert!(
                grpc.invoke(auth(invoke(json!({"value":1}))?, "manager")?)
                    .await
                    .is_err()
            );
            let before = remote.counts.calls.load(Ordering::SeqCst);
            assert!(
                grpc.invoke(auth(invoke(json!({"value":-2}))?, "remote")?)
                    .await
                    .is_err()
            );
            assert_eq!(
                remote.counts.calls.load(Ordering::SeqCst),
                before + 1,
                "上游失败不能重试"
            );
            if protocol == "mcp" {
                remote.counts.changed.store(true, Ordering::SeqCst);
                let before = remote.counts.calls.load(Ordering::SeqCst);
                assert!(
                    grpc.invoke(auth(invoke(json!({"value":5}))?, "remote")?)
                        .await
                        .is_err()
                );
                assert_eq!(remote.counts.calls.load(Ordering::SeqCst), before);
                remote.counts.changed.store(false, Ordering::SeqCst);
                for value in [-3, -4] {
                    assert!(
                        grpc.invoke(auth(invoke(json!({"value":value}))?, "remote")?)
                            .await
                            .is_err()
                    );
                }
            }
            let request = auth(invoke(json!({"value":-1}))?, "remote")?;
            let mut pending = grpc.clone();
            let task = tokio::spawn(async move { pending.invoke(request).await });
            tokio::time::timeout(Duration::from_secs(3), async {
                while remote.counts.active.load(Ordering::SeqCst) == 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await?;
            task.abort();
            tokio::time::timeout(Duration::from_secs(3), async {
                while remote.counts.active.load(Ordering::SeqCst) != 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .context("取消没有传到远端协议")?;
            let mut request = auth(invoke(json!({"value":-1}))?, "remote")?;
            request.set_timeout(Duration::from_millis(500));
            assert!(grpc.invoke(request).await.is_err());
            tokio::time::timeout(Duration::from_secs(3), async {
                while remote.counts.active.load(Ordering::SeqCst) != 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .context("截止时间没有释放上游调用")?;
        }
    }
    fixture
        .identity
        .runtime_allowed
        .store(false, Ordering::SeqCst);
    assert!(mcp.list_all_tools().await?.is_empty());
    assert!(
        grpc.invoke(auth(invoke(json!({"value":1}))?, "remote")?)
            .await
            .is_err()
    );
    fixture
        .identity
        .runtime_allowed
        .store(true, Ordering::SeqCst);
    fixture.identity.revoked.store(true, Ordering::SeqCst);
    assert!(mcp.list_all_tools().await.is_err());
    assert!(
        grpc.invoke(auth(invoke(json!({"value":1}))?, "remote")?)
            .await
            .is_err()
    );
    assert!(remote.counts.credentials.load(Ordering::SeqCst) > 0);
    assert!(remote.counts.deadlines.load(Ordering::SeqCst) > 0);
    mcp.cancel().await?;
    manager.cancel().await?;
    other.cancel().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL 和 AIO_REMOTE_TEST_CREDENTIAL=remote-test"]
async fn rejects_changed_contract_oversized_stream_and_redirect_during_import() -> Result<()> {
    let remote = Upstream::open().await?;
    for endpoint in ["oversized", "redirect"] {
        let mut import = remote.imports().remove(0);
        import["endpoint"] = json!(format!("{}/{endpoint}", remote.base));
        let error =
            Fixture::configured(vec![Arc::new(RemoteCapabilitiesPlugin)], Some(vec![import]))
                .await
                .err()
                .context("不安全的远端导入成功")?;
        let message = format!("{error:#}");
        assert!(message.contains("MCP"), "{message}");
        assert_eq!(remote.counts.calls.load(Ordering::SeqCst), 0);
    }
    remote.counts.changed.store(true, Ordering::SeqCst);
    assert!(
        Fixture::configured(
            vec![Arc::new(RemoteCapabilitiesPlugin)],
            Some(remote.imports())
        )
        .await
        .is_err()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL 和 AIO_REMOTE_TEST_CREDENTIAL=remote-test"]
async fn tenant_bindings_restore_from_configuration_and_stateful_mcp_cancels() -> Result<()> {
    use crate::runtime::server::{RuntimeState, enhancements::Enhancements};
    use az_plugin_contract::{
        CapabilityGrants, EnhancementRequest, InvocationScope, RequestContext,
    };

    let first = Upstream::open().await?;
    let second = Upstream::open().await?;
    let second_tenant = uuid::Uuid::new_v4().to_string();
    let mut import = second.imports().remove(0);
    import["tenant_id"] = json!(second_tenant);
    import["endpoint"] = json!(format!("{}/stateful", second.base));
    let fixture = Fixture::configured(
        vec![Arc::new(RemoteCapabilitiesPlugin)],
        Some(vec![first.imports().remove(0), import]),
    )
    .await?;
    let scope = InvocationScope {
        source_id: "test-background".into(),
        revision: String::new(),
        context: RequestContext {
            tenant_id: Some(second_tenant.clone()),
            ..Default::default()
        },
        grants: CapabilityGrants {
            enhancements: vec!["/v1/remote/double".into()],
            runtime: vec!["/v1/remote/mcp".into()],
            ..Default::default()
        },
    };
    let service = fixture.state.service::<Enhancements>()?;
    let revision = service
        .propose(&second_tenant, "test", package("mcp", true)?)
        .await?;
    service
        .activate(&second_tenant, "test", Some(&revision), 0, false)
        .await?
        .context("激活失败")?;
    fixture.publish(package("mcp", false)?, 0).await?;
    let request = EnhancementRequest {
        path: "/v1/remote/double".into(),
        input: json!({"value":3}),
    };
    assert_eq!(
        service.invoke(&scope, request.clone(), None).await?.output,
        json!({"value":6})
    );
    assert_eq!(first.counts.calls.load(Ordering::SeqCst), 0);
    assert_eq!(second.counts.calls.load(Ordering::SeqCst), 1);
    let mut client = EnhancementsClient::connect(fixture.base.clone()).await?;
    client
        .invoke(auth(invoke(json!({"value":3}))?, "remote")?)
        .await?;
    assert_eq!(first.counts.calls.load(Ordering::SeqCst), 1);
    assert_eq!(second.counts.calls.load(Ordering::SeqCst), 1);

    let pending_scope = scope.clone();
    let task = tokio::spawn(async move {
        service
            .invoke(
                &pending_scope,
                EnhancementRequest {
                    path: "/v1/remote/double".into(),
                    input: json!({"value":-1}),
                },
                None,
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while second.counts.active.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    task.abort();
    tokio::time::timeout(Duration::from_secs(3), async {
        while second.counts.active.load(Ordering::SeqCst) != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .context("有状态 MCP 会话未取消")?;
    // 停止插件必须等待取消通知与会话清理，不能只让后续目录变为空。
    let pending_scope = scope.clone();
    let service = fixture.state.service::<Enhancements>()?;
    let pending = tokio::spawn(async move {
        service
            .invoke(
                &pending_scope,
                EnhancementRequest {
                    path: "/v1/remote/double".into(),
                    input: json!({"value":-1}),
                },
                None,
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while second.counts.active.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    fixture.state.shutdown().await?;
    assert!(pending.await?.is_err());
    assert_eq!(second.counts.active.load(Ordering::SeqCst), 0);
    fixture.server.abort();
    let restored = RuntimeState::initialize_with_plugins(
        fixture.state.config.as_ref().clone(),
        fixture.identity.clone(),
        crate::runtime::server::plugins::default_plugins()?,
    )
    .await?;
    let result = restored
        .service::<Enhancements>()?
        .invoke(&scope, request, None)
        .await;
    restored.shutdown().await?;
    assert_eq!(result?.revision, revision);
    assert_eq!(first.counts.calls.load(Ordering::SeqCst), 1);
    assert_eq!(second.counts.calls.load(Ordering::SeqCst), 4);
    Ok(())
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL 和 AIO_REMOTE_TEST_CREDENTIAL=remote-test"]
async fn rejects_invalid_imports_before_connecting_and_never_leaks_credentials() -> Result<()> {
    let remote = Upstream::open().await?;
    for case in [
        "duplicate",
        "contract",
        "schema",
        "credential",
        "missing_method",
    ] {
        let mut imports = vec![remote.imports().remove(1)];
        match case {
            "duplicate" => imports.push(imports[0].clone()),
            "contract" => {
                let mut other = imports[0].clone();
                other["tenant_id"] = json!("other");
                other["capability"]["effect"] = json!("write");
                imports.push(other);
            }
            "schema" => {
                imports[0]["capability"]["input_schema"] =
                    json!({"$ref":"https://example.com/unknown"})
            }
            "credential" => {
                imports[0]["credential_env"] = json!("AIO_REMOTE_MISSING_TEST_CREDENTIAL")
            }
            _ => imports[0]["transport"]["method"] = json!("Missing"),
        }
        let error = Fixture::configured(vec![Arc::new(RemoteCapabilitiesPlugin)], Some(imports))
            .await
            .err()
            .context("无效导入被接受")?;
        assert!(!format!("{error:#}").contains("Bearer remote-test"));
        assert_eq!(remote.counts.credentials.load(Ordering::SeqCst), 0);
        assert_eq!(remote.counts.calls.load(Ordering::SeqCst), 0);
    }
    Ok(())
}
