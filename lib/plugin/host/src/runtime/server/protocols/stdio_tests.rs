use super::reload::{reload, twice, write_imports};
use super::*;

#[path = "stdio_support.rs"]
mod process;
use process::Upstream as Stdio;

async fn fixture(remote: &Stdio) -> Result<Fixture> {
    Fixture::configured(
        vec![Arc::new(RemoteCapabilitiesPlugin)],
        Some(vec![remote.import()]),
    )
    .await
}

async fn close(fixture: Fixture, remote: &Stdio, result: Result<()>) -> Result<()> {
    let closed = fixture.state.shutdown().await;
    fixture.server.abort();
    let reaped = remote.reaped().await;
    result?;
    closed?;
    reaped
}

#[tokio::test]
#[ignore = "需要独立 PostgreSQL 与 mcp-stdio-fixture 测试程序"]
async fn stdio_uses_shared_languages_grants_schemas_and_environment_isolation() -> Result<()> {
    let remote = Stdio::new(2)?;
    let fixture = fixture(&remote).await?;
    let result = async {
        let registry = fixture.state.service::<RuntimeCapabilities>()?;
        assert_eq!(registry.catalog(&fixture.identity.tenant)?.len(), 1);
        assert!(registry.catalog("other")?.is_empty());
        let mcp = mcp_client(&fixture.base, "remote").await?;
        let manager = mcp_client(&fixture.base, "manager").await?;
        let mut grpc = EnhancementsClient::connect(fixture.base.clone()).await?;
        for (generation, typescript) in [false, true].into_iter().enumerate() {
            let before = remote.count("json")?;
            fixture
                .publish(package("mcp", typescript)?, generation as i64)
                .await?;
            assert_eq!(remote.count("json")?, before, "候选验证不得启动真实进程");
            let tools = mcp.list_all_tools().await?;
            assert_eq!(tools.len(), 2);
            assert!(
                tools
                    .iter()
                    .any(|tool| tool.name == "runtime.v1.remote.mcp")
            );
            assert!(manager.list_all_tools().await?.is_empty());
            let response = grpc
                .invoke(auth(invoke(json!({"value":21}))?, "remote")?)
                .await?
                .into_inner();
            assert_eq!(
                serde_json::from_slice::<Value>(&response.output_json)?,
                json!({"value":42})
            );
            let response = mcp
                .call_tool(
                    rmcp::model::CallToolRequestParams::new("v1.remote.double").with_arguments(
                        json!({"input":{"value":7}})
                            .as_object()
                            .cloned()
                            .context("无效参数")?,
                    ),
                )
                .await?;
            assert_eq!(
                response.structured_content.context("缺少结果")?["output"],
                json!({"value":14})
            );
            for actor in ["manager", "other"] {
                assert!(
                    grpc.invoke(auth(invoke(json!({"value":1}))?, actor)?)
                        .await
                        .is_err()
                );
            }
            let before = remote.count("json")?;
            for input in [
                json!({"value":"bad"}),
                json!({"value":1,"tenant_id":"forged"}),
            ] {
                assert!(grpc.invoke(auth(invoke(input)?, "remote")?).await.is_err());
            }
            assert_eq!(remote.count("json")?, before, "无效参数在启动进程前拒绝");
            for value in [-2, -3, -4] {
                let before = remote.count("called")?;
                assert!(
                    grpc.invoke(auth(invoke(json!({"value":value}))?, "remote")?)
                        .await
                        .is_err()
                );
                assert_eq!(remote.count("called")?, before + 1, "失败不能重放工具");
                remote.reaped().await?;
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
        mcp.cancel().await?;
        manager.cancel().await?;
        Ok(())
    }
    .await;
    close(fixture, &remote, result).await
}

#[tokio::test]
#[ignore = "需要独立 PostgreSQL 与 mcp-stdio-fixture 测试程序"]
async fn stdio_rejects_bad_output_and_bounds_handshake_and_discovery() -> Result<()> {
    let remote = Stdio::new(2)?;
    let fixture = fixture(&remote).await?;
    let result = async {
        fixture.publish(package("mcp", true)?, 0).await?;
        let mut grpc = EnhancementsClient::connect(fixture.base.clone()).await?;
        for mode in [
            "malformed",
            "oversized",
            "flood",
            "crash",
            "schema-drift",
            "handshake-stall",
            "discovery-stall",
        ] {
            remote.mode(mode)?;
            let before = remote.count("called")?;
            let mut request = auth(invoke(json!({"value":5}))?, "remote")?;
            request.set_timeout(Duration::from_millis(750));
            let error = grpc
                .invoke(request)
                .await
                .err()
                .context("错误上游未被拒绝")?;
            assert!(!error.to_string().contains("secret-canary"));
            assert_eq!(remote.count("called")?, before, "发现失败不能调用业务工具");
            remote
                .reaped()
                .await
                .with_context(|| format!("故障模式: {mode}"))?;
        }
        remote.mode("normal")?;
        grpc.invoke(auth(invoke(json!({"value":5}))?, "remote")?)
            .await?;
        Ok(())
    }
    .await;
    close(fixture, &remote, result).await
}

#[tokio::test]
#[ignore = "需要独立 PostgreSQL 与 mcp-stdio-fixture 测试程序"]
async fn stdio_cancellation_quota_and_shutdown_reclaim_process_groups() -> Result<()> {
    let remote = Stdio::new(2)?;
    let fixture = fixture(&remote).await?;
    let result = async {
        fixture.publish(package("mcp", false)?, 0).await?;
        let grpc = EnhancementsClient::connect(fixture.base.clone()).await?;
        for (mode, event) in [
            ("handshake-stall", "json"),
            ("discovery-stall", "discovered"),
            ("normal", "called"),
        ] {
            remote.mode(mode)?;
            let before = remote.count(event)?;
            let mut client = grpc.clone();
            let request = auth(invoke(json!({"value":-6}))?, "remote")?;
            let job = tokio::spawn(async move { client.invoke(request).await });
            remote.event(event, before).await?;
            job.abort();
            assert!(job.await.is_err());
            remote.reaped().await?;
        }
        let mut calls = Vec::new();
        for _ in 0..4 {
            let before = remote.count("called")?;
            let mut client = grpc.clone();
            let request = auth(invoke(json!({"value":-6}))?, "remote")?;
            calls.push(tokio::spawn(async move { client.invoke(request).await }));
            remote.event("called", before).await?;
        }
        let before = remote.count("json")?;
        assert!(
            grpc.clone()
                .invoke(auth(invoke(json!({"value":1}))?, "remote")?)
                .await
                .is_err()
        );
        assert_eq!(remote.count("json")?, before, "满额不能再启动进程");
        for call in calls {
            call.abort();
            assert!(call.await.is_err());
        }
        remote.reaped().await?;
        grpc.clone()
            .invoke(auth(invoke(json!({"value":1}))?, "remote")?)
            .await?;
        let before = remote.count("called")?;
        let mut client = grpc.clone();
        let request = auth(invoke(json!({"value":-6}))?, "remote")?;
        let call = tokio::spawn(async move { client.invoke(request).await });
        remote.event("called", before).await?;
        remote.mode("handshake-stall")?;
        let before = remote.count("json")?;
        let mut client = grpc.clone();
        let request = auth(invoke(json!({"value":1}))?, "remote")?;
        let handshake = tokio::spawn(async move { client.invoke(request).await });
        remote.event("json", before).await?;
        fixture.state.shutdown().await?;
        assert!(call.await?.is_err());
        assert!(handshake.await?.is_err());
        remote.reaped().await?;
        Ok(())
    }
    .await;
    close(fixture, &remote, result).await
}

#[tokio::test]
#[ignore = "需要独立 PostgreSQL 与 mcp-stdio-fixture 测试程序"]
async fn stdio_validates_entire_candidate_before_launch_and_keeps_active_binding() -> Result<()> {
    let remote = Stdio::new(2)?;
    let fixture = fixture(&remote).await?;
    let result = async {
        fixture.publish(package("mcp", false)?, 0).await?;
        let started = remote.count("json")?;
        let mut invalids = Vec::new();
        let original = remote.import();
        for (key, value) in [("program", json!("relative")), ("cwd", json!("relative")), ("env", json!({"CHILD_TOKEN":"MISSING_AIO_STDIO_TEST_VARIABLE"})), ("args", json!(["\0"]))] {
            let mut invalid = original.clone();
            invalid["transport"]["command"][key] = value;
            invalids.push(invalid);
        }
        for (key, value) in [("endpoint", json!("http://127.0.0.1:1")), ("credential_env", json!("AIO_REMOTE_TEST_CREDENTIAL"))] {
            let mut invalid = original.clone();
            invalid[key] = value;
            invalids.push(invalid);
        }
        let mut invalid = original.clone();
        invalid["transport"]["tool"] = json!("");
        invalids.push(invalid);
        let descriptor = remote.root.path().join("bad.bin");
        std::fs::write(&descriptor, "not-protobuf")?;
        let mut invalid = original.clone();
        invalid["endpoint"] = json!("http://127.0.0.1:1");
        invalid["transport"] = json!({"protocol":"grpc","descriptor_set":descriptor,"service":"invalid","method":"invalid"});
        invalids.push(invalid);
        for mut invalid in invalids {
            invalid["tenant_id"] = json!("other");
            write_imports(&fixture, vec![original.clone(), invalid]).await?;
            assert!(reload(&fixture).await.is_err());
            assert_eq!(remote.count("json")?, started, "本地校验未完成就启动了前面的进程");
        }
        // 部署候选没有调用方的短截止时间时，握手本身仍须有界，并完整回收失败候选。
        let stalled = Stdio::new(2)?;
        stalled.mode("handshake-stall")?;
        write_imports(&fixture, vec![stalled.import()]).await?;
        let error = tokio::time::timeout(Duration::from_secs(8), reload(&fixture)).await?
            .err().context("挂起握手没有失败")?;
        assert!(format!("{error:#}").contains("MCP 握手超时"));
        // 撤回坏候选并等待同一加载锁，避免观察器重试启动新进程时误判旧进程泄漏。
        write_imports(&fixture, vec![original]).await?;
        assert!(!reload(&fixture).await?);
        stalled.reaped().await?;
        let mut grpc = EnhancementsClient::connect(fixture.base.clone()).await?;
        let response = grpc.invoke(auth(invoke(json!({"value":7}))?, "remote")?).await?.into_inner();
        assert_eq!(serde_json::from_slice::<Value>(&response.output_json)?, json!({"value":14}));
        Ok(())
    }.await;
    close(fixture, &remote, result).await
}

#[tokio::test]
#[ignore = "需要独立 PostgreSQL 与 mcp-stdio-fixture 测试程序"]
async fn stdio_reload_pins_running_script_and_rejects_failed_candidate() -> Result<()> {
    let first = Stdio::new(2)?;
    let second = Stdio::new(3)?;
    let fixture = fixture(&first).await?;
    let result = async {
        let mut grpc = EnhancementsClient::connect(fixture.base.clone()).await?;
        for (generation, typescript) in [false, true].into_iter().enumerate() {
            write_imports(&fixture, vec![first.import()]).await?;
            reload(&fixture).await?;
            if first.root.path().join("release").exists() {
                std::fs::remove_file(first.root.path().join("release"))?;
            }
            fixture
                .publish(twice("mcp", typescript)?, generation as i64)
                .await?;
            let before = first.count("called")?;
            let mut client = grpc.clone();
            let request = auth(invoke(json!({"value":-5}))?, "remote")?;
            let running = tokio::spawn(async move { client.invoke(request).await });
            first.event("called", before).await?;
            write_imports(&fixture, vec![second.import()]).await?;
            assert!(reload(&fixture).await?);
            std::fs::write(first.root.path().join("release"), "")?;
            let response = running.await??.into_inner();
            assert_eq!(
                serde_json::from_slice::<Value>(&response.output_json)?,
                json!({"value":-20}),
                "调用中的脚本混用了新绑定"
            );
            let response = grpc
                .invoke(auth(invoke(json!({"value":7}))?, "remote")?)
                .await?
                .into_inner();
            assert_eq!(
                serde_json::from_slice::<Value>(&response.output_json)?,
                json!({"value":63})
            );
            first.mode("schema-drift")?;
            write_imports(&fixture, vec![first.import()]).await?;
            assert!(reload(&fixture).await.is_err());
            let response = grpc
                .invoke(auth(invoke(json!({"value":7}))?, "remote")?)
                .await?
                .into_inner();
            assert_eq!(
                serde_json::from_slice::<Value>(&response.output_json)?,
                json!({"value":63})
            );
            first.mode("normal")?;
        }
        write_imports(&fixture, vec![]).await?;
        assert!(reload(&fixture).await?);
        assert!(
            grpc.invoke(auth(invoke(json!({"value":7}))?, "remote")?)
                .await
                .is_err()
        );
        second.reaped().await?;
        Ok(())
    }
    .await;
    close(fixture, &first, result).await?;
    second.reaped().await
}
