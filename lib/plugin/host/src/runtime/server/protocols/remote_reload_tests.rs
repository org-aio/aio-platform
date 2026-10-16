use super::*;
use crate::runtime::server::{
    enhancements::Enhancements, plugins::CapabilityProvider, remote::RemoteCapabilities,
};
use az_plugin_contract::{CapabilityGrants, EnhancementRequest, InvocationScope, RequestContext};

fn twice(protocol: &str, typescript: bool) -> Result<Package> {
    let path = format!("/v1/remote/{protocol}");
    let source = if typescript {
        format!(
            "export function invoke(input) {{ const first = call_capability('{path}', input); return call_capability('{path}', first); }}"
        )
    } else {
        format!(
            "fn invoke(input, context) {{ let first = call_capability(\"{path}\", input); call_capability(\"{path}\", first) }}"
        )
    };
    Ok(serde_json::from_value(
        json!({"schema_version":1,"enhancements":[{
            "path":"/v1/remote/double","language":if typescript {"typescript"} else {"rhai"},"source":source,"capabilities":[path],
            "examples":[{"input":{"value":3},"output":{"value":12},"capabilities":[
                {"path":path,"input":{"value":3},"output":{"value":6}},
                {"path":path,"input":{"value":6},"output":{"value":12}}
            ]}]
        }]}),
    )?)
}

fn config_path(fixture: &Fixture) -> Result<&std::path::Path> {
    fixture
        .state
        .config
        .remote_capabilities
        .as_deref()
        .context("缺少测试配置")
}

async fn write_imports(fixture: &Fixture, mut imports: Vec<Value>) -> Result<()> {
    for import in &mut imports {
        if import["tenant_id"] == "test" {
            import["tenant_id"] = json!(fixture.identity.tenant);
        }
    }
    let path = config_path(fixture)?;
    let pending = path.with_extension("pending");
    tokio::fs::write(
        &pending,
        serde_json::to_vec(&json!({"schema_version":1,"imports":imports}))?,
    )
    .await?;
    tokio::fs::rename(pending, path).await?;
    Ok(())
}

async fn observed_update(fixture: &Fixture, imports: Vec<Value>) -> Result<()> {
    let provider = fixture.state.service::<RemoteCapabilities>()?;
    let before = provider.snapshot().context("缺少能力快照")?;
    write_imports(fixture, imports).await?;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let next = provider.snapshot().context("缺少新快照")?;
            if !Arc::ptr_eq(&before, &next) {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .context("文件观察器没有更新能力")??;
    Ok(())
}

async fn reload(fixture: &Fixture) -> Result<bool> {
    fixture
        .state
        .service::<RemoteCapabilities>()?
        .reload(
            config_path(fixture)?,
            fixture.state.service::<RuntimeCapabilities>()?.as_ref(),
        )
        .await
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL 和 AIO_REMOTE_TEST_CREDENTIAL=remote-test"]
async fn running_scripts_pin_bindings_while_new_calls_and_discovery_observe_updates() -> Result<()>
{
    let first = Upstream::open().await?;
    let second = Upstream::open().await?;
    let fixture =
        Fixture::configured(vec![Arc::new(RemoteCapabilitiesPlugin)], Some(vec![])).await?;
    let result = tokio::time::timeout(
        Duration::from_secs(90),
        exercise_updates(&fixture, &first, &second),
    )
    .await;
    let cleanup = fixture.state.shutdown().await;
    fixture.server.abort();
    result??;
    cleanup?;
    assert!(reload(&fixture).await.is_err(), "停用后不能重新发布绑定");
    Ok(())
}

async fn exercise_updates(fixture: &Fixture, first: &Upstream, second: &Upstream) -> Result<()> {
    let registry = fixture.state.service::<RuntimeCapabilities>()?;
    assert!(registry.catalog(&fixture.identity.tenant)?.is_empty());
    let other_tenant = format!("{}-other", fixture.identity.tenant);
    let mut others = first.imports();
    for import in &mut others {
        import["tenant_id"] = json!(other_tenant);
    }
    let original: Vec<_> = first.imports().into_iter().chain(others.clone()).collect();
    let changed: Vec<_> = second.imports().into_iter().chain(others.clone()).collect();
    observed_update(fixture, original.clone()).await?;
    assert_eq!(registry.catalog(&fixture.identity.tenant)?.len(), 2);
    assert_eq!(registry.catalog(&other_tenant)?.len(), 2);
    let mcp = mcp_client(&fixture.base, "remote").await?;
    let mut grpc = EnhancementsClient::connect(fixture.base.clone()).await?;
    let service = fixture.state.service::<Enhancements>()?;
    let mut generation = 0;
    let mut other_generation = 0;
    for protocol in ["mcp", "grpc"] {
        for typescript in [false, true] {
            if protocol == "grpc" && !typescript {
                for (tenant, previous) in [
                    (&fixture.identity.tenant, generation),
                    (&other_tenant, other_generation),
                ] {
                    service
                        .activate(tenant, "test", None, previous, false)
                        .await?
                        .context("停用失败")?;
                }
                generation += 1;
                other_generation += 1;
            }
            write_imports(fixture, original.clone()).await?;
            reload(fixture).await?;
            fixture
                .publish(twice(protocol, typescript)?, generation)
                .await?;
            generation += 1;
            let revision = service
                .propose(&other_tenant, "test", twice(protocol, typescript)?)
                .await?;
            service
                .activate(
                    &other_tenant,
                    "test",
                    Some(&revision),
                    other_generation,
                    false,
                )
                .await?
                .context("激活失败")?;
            other_generation += 1;
            let old_calls = first.counts.calls.load(Ordering::SeqCst);
            let new_calls = second.counts.calls.load(Ordering::SeqCst);
            let mut pending = grpc.clone();
            let request = auth(invoke(json!({"value":-5}))?, "remote")?;
            let pending = tokio::spawn(async move { pending.invoke(request).await });
            tokio::time::timeout(Duration::from_secs(3), async {
                while first.counts.active.load(Ordering::SeqCst) == 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await?;
            // 第一条上游请求仍未返回；切换后，该脚本的第二次调用也必须留在旧上游。
            observed_update(fixture, changed.clone()).await?;
            assert_eq!(first.counts.calls.load(Ordering::SeqCst), old_calls + 1);
            assert_eq!(
                second.counts.calls.load(Ordering::SeqCst),
                new_calls,
                "候选校验不能执行工具"
            );
            assert_eq!(mcp.list_all_tools().await?.len(), 1);
            let response = grpc
                .invoke(auth(invoke(json!({"value":7}))?, "remote")?)
                .await?
                .into_inner();
            assert_eq!(
                serde_json::from_slice::<Value>(&response.output_json)?,
                json!({"value":28})
            );
            let response = mcp
                .call_tool(
                    rmcp::model::CallToolRequestParams::new("v1.remote.double").with_arguments(
                        json!({"input":{"value":8}})
                            .as_object()
                            .cloned()
                            .context("参数不是对象")?,
                    ),
                )
                .await?;
            assert_eq!(
                response.structured_content.context("缺少结果")?["output"],
                json!({"value":32})
            );
            assert_eq!(second.counts.calls.load(Ordering::SeqCst), new_calls + 4);
            first.counts.release.notify_one();
            let response = tokio::time::timeout(Duration::from_secs(3), pending)
                .await???
                .into_inner();
            assert_eq!(
                serde_json::from_slice::<Value>(&response.output_json)?,
                json!({"value":-20})
            );
            assert_eq!(first.counts.calls.load(Ordering::SeqCst), old_calls + 2);
            let scope = InvocationScope {
                source_id: "test-background".into(),
                revision: String::new(),
                context: RequestContext {
                    tenant_id: Some(other_tenant.clone()),
                    ..Default::default()
                },
                grants: CapabilityGrants {
                    enhancements: vec!["/v1/remote/double".into()],
                    runtime: vec![format!("/v1/remote/{protocol}")],
                    ..Default::default()
                },
            };
            let response = service
                .invoke(
                    &scope,
                    EnhancementRequest {
                        path: "/v1/remote/double".into(),
                        input: json!({"value":9}),
                    },
                    None,
                )
                .await?;
            assert_eq!(response.output, json!({"value":36}));
            assert_eq!(
                first.counts.calls.load(Ordering::SeqCst),
                old_calls + 4,
                "更新不能串到另一个租户"
            );
        }
    }
    observed_update(fixture, others).await?;
    assert!(registry.catalog(&fixture.identity.tenant)?.is_empty());
    assert_eq!(registry.catalog(&other_tenant)?.len(), 2);
    assert!(mcp.list_all_tools().await?.is_empty());
    assert!(
        grpc.invoke(auth(invoke(json!({"value":1}))?, "remote")?)
            .await
            .is_err()
    );
    mcp.cancel().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL 和 AIO_REMOTE_TEST_CREDENTIAL=remote-test"]
async fn rejected_updates_preserve_live_bindings_and_descriptor_changes_are_detected() -> Result<()>
{
    let remote = Upstream::open().await?;
    let fixture = Fixture::configured(
        vec![Arc::new(RemoteCapabilitiesPlugin)],
        Some(remote.imports()),
    )
    .await?;
    let result = tokio::time::timeout(
        Duration::from_secs(45),
        exercise_rejections(&fixture, &remote),
    )
    .await;
    let cleanup = fixture.state.shutdown().await;
    fixture.server.abort();
    result??;
    cleanup?;
    Ok(())
}

async fn exercise_rejections(fixture: &Fixture, remote: &Upstream) -> Result<()> {
    let provider = fixture.state.service::<RemoteCapabilities>()?;
    let registry = fixture.state.service::<RuntimeCapabilities>()?;
    fixture.publish(package("grpc", true)?, 0).await?;
    let mut grpc = EnhancementsClient::connect(fixture.base.clone()).await?;
    let original = provider.snapshot().context("缺少快照")?;
    tokio::fs::write(config_path(fixture)?, b"{").await?;
    assert!(reload(fixture).await.is_err());
    for invalid in 0..3 {
        let mut imports = remote.imports();
        match invalid {
            0 => imports[1]["capability"]["effect"] = json!("write"),
            1 => imports.push(imports[0].clone()),
            _ => {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
                imports[1]["endpoint"] = json!(format!("http://{}", listener.local_addr()?));
                drop(listener);
            }
        }
        write_imports(fixture, imports).await?;
        assert!(reload(fixture).await.is_err(), "无效更新被接受: {invalid}");
        assert!(Arc::ptr_eq(
            &original,
            &provider.snapshot().context("缺少快照")?
        ));
        assert_eq!(registry.catalog(&fixture.identity.tenant)?.len(), 2);
        let response = grpc
            .invoke(auth(invoke(json!({"value":7}))?, "remote")?)
            .await?
            .into_inner();
        assert_eq!(
            serde_json::from_slice::<Value>(&response.output_json)?,
            json!({"value":14})
        );
    }
    write_imports(fixture, vec![]).await?;
    assert!(reload(fixture).await?);
    let mut incompatible = remote.imports();
    incompatible[1]["capability"]["effect"] = json!("write");
    write_imports(fixture, incompatible).await?;
    assert!(reload(fixture).await.is_err(), "删除不能绕过历史契约");
    assert!(
        registry.catalog(&fixture.identity.tenant)?.is_empty(),
        "失败不能部分激活 MCP 绑定"
    );
    write_imports(fixture, remote.imports()).await?;
    assert!(reload(fixture).await?);
    assert!(!reload(fixture).await?, "相同文件不应反复建连");
    let imports = remote.imports();
    let descriptor_path = imports[1]["transport"]["descriptor_set"]
        .as_str()
        .context("缺少描述文件")?;
    let bytes = tokio::fs::read(descriptor_path).await?;
    tokio::fs::write(descriptor_path, b"invalid").await?;
    assert!(reload(fixture).await.is_err());
    let response = grpc
        .invoke(auth(invoke(json!({"value":4}))?, "remote")?)
        .await?
        .into_inner();
    assert_eq!(
        serde_json::from_slice::<Value>(&response.output_json)?,
        json!({"value":8})
    );
    use prost::Message;
    let mut descriptor = prost_types::FileDescriptorSet::decode(bytes.as_slice())?;
    descriptor.file[0].name = Some("updated.proto".into());
    tokio::fs::write(descriptor_path, descriptor.encode_to_vec()).await?;
    assert!(reload(fixture).await?, "只修改描述文件也必须触发更新");
    let response = grpc
        .invoke(auth(invoke(json!({"value":5}))?, "remote")?)
        .await?
        .into_inner();
    assert_eq!(
        serde_json::from_slice::<Value>(&response.output_json)?,
        json!({"value":10})
    );
    Ok(())
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL 和 AIO_REMOTE_TEST_CREDENTIAL=remote-test"]
async fn obsolete_candidates_never_publish_and_shutdown_cancels_discovery() -> Result<()> {
    let first = Upstream::open().await?;
    let second = Upstream::open().await?;
    let fixture = Fixture::configured(
        vec![Arc::new(RemoteCapabilitiesPlugin)],
        Some(first.imports()),
    )
    .await?;
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        exercise_obsolete(&fixture, &first, &second),
    )
    .await;
    let cleanup = fixture.state.shutdown().await;
    fixture.server.abort();
    result??;
    cleanup?;
    Ok(())
}

async fn exercise_obsolete(fixture: &Fixture, first: &Upstream, second: &Upstream) -> Result<()> {
    let provider = fixture.state.service::<RemoteCapabilities>()?;
    let original = provider.snapshot().context("缺少快照")?;
    let registry = fixture.state.service::<RuntimeCapabilities>()?;
    second.counts.pause_catalog.store(true, Ordering::SeqCst);
    write_imports(fixture, second.imports()).await?;
    let pending_provider = provider.clone();
    let pending_registry = registry.clone();
    let path = config_path(fixture)?.to_owned();
    let pending =
        tokio::spawn(async move { pending_provider.reload(&path, &pending_registry).await });
    tokio::time::timeout(
        Duration::from_secs(5),
        second.counts.catalog_entered.notified(),
    )
    .await?;
    write_imports(fixture, first.imports()).await?;
    second.counts.pause_catalog.store(false, Ordering::SeqCst);
    second.counts.catalog_release.notify_one();
    match pending.await? {
        Err(error) => assert!(format!("{error:#}").contains("文件发生变化"), "{error:#}"),
        // 若观察器先取得锁，它会拒绝过期候选；随后显式检查发现磁盘已回到活动版本。
        Ok(changed) => assert!(!changed, "过期候选被发布"),
    }
    assert!(Arc::ptr_eq(
        &original,
        &provider.snapshot().context("缺少快照")?
    ));
    assert_eq!(second.counts.calls.load(Ordering::SeqCst), 0);

    // 由真实后台观察器进入阻塞的发现请求，再关闭宿主；无需等待完整的 30 秒导入超时。
    second.counts.pause_catalog.store(true, Ordering::SeqCst);
    write_imports(fixture, second.imports()).await?;
    tokio::time::timeout(
        Duration::from_secs(10),
        second.counts.catalog_entered.notified(),
    )
    .await?;
    tokio::time::timeout(Duration::from_secs(3), fixture.state.shutdown()).await??;
    assert!(registry.catalog(&fixture.identity.tenant)?.is_empty());
    assert!(reload(fixture).await.is_err());
    assert_eq!(second.counts.calls.load(Ordering::SeqCst), 0);
    Ok(())
}
