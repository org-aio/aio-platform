use anyhow::{Context, Result};
use az_plugin_contract::grpc::{
    DescribeRequest, InvokeRequest, enhancements_client::EnhancementsClient,
};
use az_plugin_enhancement::{Language, Package};
use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};

use super::support::*;
use crate::runtime::server::enhancements::{Activation, Enhancements};

fn typed(language: Language) -> Result<Package> {
    let mut value = package(language);
    value.enhancements[0].input_schema = Some(
        json!({
            "$defs":{"number":{"type":"integer","minimum":0}}, "$ref":"#/$defs/number"
        })
        .try_into()?,
    );
    value.enhancements[0].output_schema = Some(
        json!({
            "$defs":{"number":{"type":"integer","maximum":10}}, "$ref":"#/$defs/number"
        })
        .try_into()?,
    );
    Ok(value)
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn protocols_expose_and_enforce_persistent_schema_contracts() -> Result<()> {
    let fixture = Fixture::open().await?;
    let outcome =
        tokio::time::timeout(std::time::Duration::from_secs(60), exercise(&fixture)).await;
    fixture.server.abort();
    outcome??;
    Ok(())
}

async fn exercise(fixture: &Fixture) -> Result<()> {
    let package = typed(Language::Rhai)?;
    let input = serde_json::to_value(&package.enhancements[0].input_schema)?;
    let output = serde_json::to_value(&package.enhancements[0].output_schema)?;
    let first = fixture.publish(package, 0).await?;
    let mut mcp = mcp_client(&fixture.base, "limited").await?;
    let mut grpc = EnhancementsClient::connect(fixture.base.clone()).await?;
    let tools = mcp.list_all_tools().await?;
    let tool = tools.first().context("缺少增强工具")?;
    let input_validator = jsonschema::options()
        .offline()
        .build(&json!(tool.input_schema))?;
    let output_validator = jsonschema::options()
        .offline()
        .build(&json!(tool.output_schema))?;
    // 局部引用必须仍指向业务 schema，而不是外层 MCP 参数对象。
    assert!(input_validator.is_valid(&json!({"input":4})));
    assert!(!input_validator.is_valid(&json!({"input":-1})));
    assert!(!input_validator.is_valid(&json!({"input":"private-data"})));
    assert!(!input_validator.is_valid(&json!({"input":4,"tenant":"foreign"})));
    assert!(output_validator.is_valid(&json!({"revision":first,"output":8})));
    assert!(!output_validator.is_valid(&json!({"revision":first,"output":14})));
    let contract = grpc
        .describe(auth(DescribeRequest { path: PATH.into() }, "limited")?)
        .await?
        .into_inner();
    assert_eq!(contract.revision, first);
    assert_eq!(
        serde_json::from_slice::<Value>(&contract.input_schema_json)?,
        input
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&contract.output_schema_json)?,
        output
    );
    for actor in ["denied", "other"] {
        assert_eq!(
            grpc.describe(auth(DescribeRequest { path: PATH.into() }, actor)?)
                .await
                .err()
                .context("越权发现被接受")?
                .code(),
            tonic::Code::NotFound
        );
    }
    for input in [json!(-1), json!("private-data"), json!(7)] {
        let result = mcp
            .call_tool(
                CallToolRequestParams::new("v1.math.double").with_arguments(
                    json!({"input":input})
                        .as_object()
                        .context("参数不是对象")?
                        .clone(),
                ),
            )
            .await?;
        assert_eq!(result.is_error, Some(true));
        assert!(!serde_json::to_string(&result)?.contains("private-data"));
        let request = InvokeRequest {
            path: PATH.into(),
            input_json: serde_json::to_vec(&input)?,
        };
        assert_eq!(
            grpc.invoke(auth(request, "limited")?)
                .await
                .err()
                .context("无效输入或输出被接受")?
                .code(),
            tonic::Code::FailedPrecondition
        );
    }
    let valid = mcp
        .call_tool(
            CallToolRequestParams::new("v1.math.double").with_arguments(
                json!({"input":4})
                    .as_object()
                    .context("参数不是对象")?
                    .clone(),
            ),
        )
        .await?;
    assert!(output_validator.is_valid(&valid.structured_content.context("缺少输出")?));
    let second = fixture.publish(typed(Language::TypeScript)?, 1).await?;
    assert_eq!(
        grpc.describe(auth(DescribeRequest { path: PATH.into() }, "limited")?)
            .await?
            .into_inner()
            .revision,
        second
    );

    let mut changed = typed(Language::TypeScript)?;
    changed.enhancements[0].input_schema =
        Some(json!({"type":"integer","minimum":0,"maximum":10}).try_into()?);
    let http = reqwest::Client::new();
    let candidate: Value = http
        .post(format!(
            "{}/api/runtime/enhancements/revisions",
            fixture.base
        ))
        .bearer_auth("manager")
        .json(&changed)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let rejected = http
        .put(format!("{}/api/runtime/enhancements/active", fixture.base))
        .bearer_auth("manager")
        .json(&json!({"revision":candidate["revision"],"expected_generation":2}))
        .send()
        .await?;
    assert_eq!(rejected.status(), 400);
    http.put(format!("{}/api/runtime/enhancements/active", fixture.base))
        .bearer_auth("manager")
        .json(&json!({"revision":null,"expected_generation":2}))
        .send()
        .await?
        .error_for_status()?;
    assert!(mcp.list_all_tools().await?.is_empty());
    let rejected = http
        .put(format!("{}/api/runtime/enhancements/active", fixture.base))
        .bearer_auth("manager")
        .json(&json!({"revision":candidate["revision"],"expected_generation":3}))
        .send()
        .await?;
    assert_eq!(rejected.status(), 400, "停用不能绕过历史契约约束");
    let service = fixture.state.service::<Enhancements>()?;
    assert_eq!(service.head(&fixture.identity.tenant).await?.generation, 3);
    http.post(format!(
        "{}/api/runtime/enhancements/rollback",
        fixture.base
    ))
    .bearer_auth("manager")
    .json(&json!({"revision":first,"expected_generation":3}))
    .send()
    .await?
    .error_for_status()?;
    let restored = grpc
        .describe(auth(DescribeRequest { path: PATH.into() }, "limited")?)
        .await?
        .into_inner();
    assert_eq!(restored.revision, first);
    assert_eq!(
        serde_json::from_slice::<Value>(&restored.input_schema_json)?,
        input
    );
    let fresh = Enhancements::open(
        sqlx::PgPool::connect(&std::env::var("AIO_ENHANCEMENT_TEST_DATABASE_URL")?).await?,
        az_plugin_enhancement::default_runtime()?,
        None,
    )
    .await?;
    let (revision, contracts) = fresh
        .catalog(&fixture.identity.tenant, |_| true, |_| true)
        .await?;
    assert_eq!(revision.as_deref(), Some(first.as_str()));
    let contract = contracts
        .iter()
        .find(|contract| contract.path == PATH)
        .context("重启丢失契约")?;
    assert_eq!(serde_json::to_value(&contract.input_schema)?, input);
    // 回滚可隐藏新路径，但不能释放其已发布契约。
    let mut expanded = typed(Language::Rhai)?;
    let mut extra = expanded.enhancements[0].clone();
    extra.path = "/v1/math/extra".into();
    expanded.enhancements.push(extra);
    fixture.publish(expanded.clone(), 4).await?;
    service
        .activate(
            &fixture.identity.tenant,
            "manager",
            Activation {
                revision: Some(first),
                expected_generation: 5,
            },
            true,
            async { Ok(()) },
        )
        .await?
        .context("回滚失败")?;
    expanded.enhancements[2].output_schema =
        Some(json!({"type":"integer","maximum":9}).try_into()?);
    let incompatible = service
        .propose(&fixture.identity.tenant, "manager", expanded, async {
            Ok(())
        })
        .await?;
    assert!(
        service
            .activate(
                &fixture.identity.tenant,
                "manager",
                Activation {
                    revision: Some(incompatible),
                    expected_generation: 6
                },
                false,
                async { Ok(()) },
            )
            .await
            .is_err()
    );
    assert_eq!(service.head(&fixture.identity.tenant).await?.generation, 6);
    // 契约保留只约束本租户，其他租户可以拥有自己的同名路径。
    let mut foreign = typed(Language::Rhai)?;
    foreign.enhancements[0].input_schema = Some(json!({"type":"integer"}).try_into()?);
    let tenant = format!("{}-other", fixture.identity.tenant);
    let revision = service
        .propose(&tenant, "other", foreign, async { Ok(()) })
        .await?;
    service
        .activate(
            &tenant,
            "other",
            Activation {
                revision: Some(revision),
                expected_generation: 0,
            },
            false,
            async { Ok(()) },
        )
        .await?
        .context("另一租户不能发布独立契约")?;
    let foreign = grpc
        .describe(auth(DescribeRequest { path: PATH.into() }, "other")?)
        .await?
        .into_inner();
    assert_eq!(
        serde_json::from_slice::<Value>(&foreign.input_schema_json)?,
        json!({"type":"integer"})
    );
    let own = grpc
        .describe(auth(DescribeRequest { path: PATH.into() }, "limited")?)
        .await?
        .into_inner();
    assert_eq!(
        serde_json::from_slice::<Value>(&own.input_schema_json)?,
        input
    );
    mcp.close().await?;
    Ok(())
}
