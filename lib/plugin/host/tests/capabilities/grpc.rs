use az_plugin_contract::grpc::{
    CapabilityEffect, DescribeRequest, InvokeRequest, ListRequest,
    capabilities_client::CapabilitiesClient,
};
use tonic::{Code, Request};

use super::*;

fn auth<T>(value: T, actor: &str) -> Result<Request<T>> {
    let mut request = Request::new(value);
    request
        .metadata_mut()
        .insert("authorization", format!("Bearer {actor}").parse()?);
    Ok(request)
}

fn input(value: Value) -> Result<InvokeRequest> {
    Ok(InvokeRequest {
        path: PATH.into(),
        input_json: serde_json::to_vec(&value)?,
    })
}

#[tokio::test]
async fn grpc_native_discovery_execution_revocation_and_lifecycle_need_no_interpreter() -> Result<()>
{
    let mut fixture = Fixture::open(Identity::default(), vec![]).await?;
    let mut client = CapabilitiesClient::connect(fixture.base.clone()).await?;
    let mut enhancements =
        az_plugin_contract::grpc::enhancements_client::EnhancementsClient::connect(
            fixture.base.clone(),
        )
        .await?;
    assert_eq!(
        enhancements
            .list(auth(ListRequest {}, "manager")?)
            .await
            .err()
            .context("未安装的增强服务仍可访问")?
            .code(),
        Code::Unimplemented
    );
    assert_eq!(
        client
            .list(ListRequest {})
            .await
            .err()
            .context("匿名请求通过")?
            .code(),
        Code::Unauthenticated
    );
    for actor in ["limited", "manager", "other"] {
        assert!(
            client
                .list(auth(ListRequest {}, actor)?)
                .await?
                .into_inner()
                .paths
                .is_empty()
        );
        assert_eq!(
            client
                .describe(auth(DescribeRequest { path: PATH.into() }, actor)?)
                .await
                .err()
                .context("越权契约被暴露")?
                .code(),
            Code::NotFound
        );
        assert!(
            client
                .invoke(auth(input(Value::Null)?, actor)?)
                .await
                .is_err()
        );
    }
    assert_eq!(fixture.probe.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        client
            .list(auth(ListRequest {}, "capable")?)
            .await?
            .into_inner()
            .paths,
        [PATH]
    );
    let contract = client
        .describe(auth(DescribeRequest { path: PATH.into() }, "capable")?)
        .await?
        .into_inner();
    assert_eq!(contract.path, PATH);
    assert_eq!(contract.effect, CapabilityEffect::Write as i32);
    let schema: Value = serde_json::from_slice(&contract.output_schema_json)?;
    assert_eq!(schema["type"], "integer");
    let result = client
        .invoke(auth(input(json!(21))?, "capable")?)
        .await?
        .into_inner();
    assert_eq!(serde_json::from_slice::<Value>(&result.output_json)?, 42);
    fixture.assert_scope()?;

    for value in [
        json!({"tenant_id":"other","source_id":"admin"}),
        json!("invalid"),
        json!("large"),
        json!("fail"),
    ] {
        assert!(
            client
                .invoke(auth(input(value)?, "capable")?)
                .await
                .is_err()
        );
    }
    assert_eq!(fixture.probe.calls.load(Ordering::SeqCst), 4);
    let mut malformed = input(Value::Null)?;
    malformed.input_json = b"not JSON".to_vec();
    assert_eq!(
        client
            .invoke(auth(malformed, "capable")?)
            .await
            .err()
            .context("非法 JSON 通过")?
            .code(),
        Code::InvalidArgument
    );
    assert!(
        client
            .invoke(auth(input(json!("x".repeat(65536)))?, "capable")?)
            .await
            .is_err()
    );
    assert_eq!(fixture.probe.calls.load(Ordering::SeqCst), 4);

    fixture
        .identity
        .runtime_revoked
        .store(true, Ordering::SeqCst);
    assert!(
        client
            .list(auth(ListRequest {}, "capable")?)
            .await?
            .into_inner()
            .paths
            .is_empty()
    );
    assert_eq!(
        client
            .invoke(auth(input(Value::Null)?, "capable")?)
            .await
            .err()
            .context("撤权未生效")?
            .code(),
        Code::PermissionDenied
    );
    fixture
        .identity
        .runtime_revoked
        .store(false, Ordering::SeqCst);
    fixture.identity.removed.store(true, Ordering::SeqCst);
    assert!(client.list(auth(ListRequest {}, "capable")?).await.is_err());
    fixture.identity.removed.store(false, Ordering::SeqCst);
    fixture.identity.revoked.store(true, Ordering::SeqCst);
    assert_eq!(
        client
            .list(auth(ListRequest {}, "capable")?)
            .await
            .err()
            .context("会话撤销未生效")?
            .code(),
        Code::Unauthenticated
    );
    fixture.identity.revoked.store(false, Ordering::SeqCst);

    fixture.plugins[1] = Arc::new(ProviderPlugin(fixture.probe.clone(), 84));
    fixture.host.reconfigure(fixture.plugins.clone()).await?;
    let result = client
        .invoke(auth(input(Value::Null)?, "capable")?)
        .await?
        .into_inner();
    assert_eq!(serde_json::from_slice::<Value>(&result.output_json)?, 84);
    fixture
        .plugins
        .retain(|plugin| plugin.as_ref().type_id() != TypeId::of::<GrpcPlugin>());
    fixture.host.reconfigure(fixture.plugins.clone()).await?;
    assert!(client.list(auth(ListRequest {}, "capable")?).await.is_err());
    fixture.plugins.push(Arc::new(GrpcPlugin));
    fixture.host.reconfigure(fixture.plugins.clone()).await?;
    assert_eq!(
        client
            .list(auth(ListRequest {}, "capable")?)
            .await?
            .into_inner()
            .paths,
        [PATH]
    );
    fixture.close().await
}

#[tokio::test]
async fn grpc_deadline_cancels_native_future_before_resource_shutdown() -> Result<()> {
    let fixture = Fixture::open(Identity::default(), vec![]).await?;
    let mut client = CapabilitiesClient::connect(fixture.base.clone()).await?;
    let mut request = auth(input(json!("wait"))?, "capable")?;
    request.set_timeout(Duration::from_millis(150));
    let result = tokio::time::timeout(Duration::from_secs(3), client.invoke(request)).await?;
    // Tonic 也会在客户端执行 grpc-timeout；本地取消可能先于服务器的截止时间响应。
    let status = result.err().context("截止时间未生效")?;
    assert!(
        matches!(status.code(), Code::DeadlineExceeded | Code::Cancelled),
        "{status}"
    );
    assert_eq!(fixture.probe.calls.load(Ordering::SeqCst), 1);
    tokio::time::timeout(Duration::from_secs(3), async {
        while fixture.probe.active.load(Ordering::SeqCst) != 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    fixture.close().await
}
