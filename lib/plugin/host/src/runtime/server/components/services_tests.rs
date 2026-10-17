use super::*;

fn session() -> SessionContext {
    SessionContext {
        tenant_id: "tenant".into(),
        user_id: "user".into(),
        session_id: "session".into(),
        account: "account".into(),
        display_name: "User".into(),
        tenant_label: "Tenant".into(),
        permissions: vec![permission("first", "read"), permission("second", "read")],
    }
}

fn scope(context: &RequestContext, source: &str) -> InvocationScope {
    InvocationScope {
        source_id: source.into(),
        revision: "revision".into(),
        context: context.clone(),
        grants: Default::default(),
    }
}

#[tokio::test]
async fn interactive_requests_bind_source_and_delegation_expires_with_parent() -> Result<()> {
    let services = Arc::new(Services::with_enhancements(None));
    let request = services.enter(&session(), "first")?;
    let id = &request.context.request_id;
    assert!(services.interactive(id, "tenant", "first").is_ok());
    assert!(services.interactive(id, "other", "first").is_err());
    assert!(services.interactive(id, "tenant", "second").is_err());
    assert!(
        services
            .authorize(&scope(&request.context, "first"), "read")
            .await?
    );
    let error = services
        .enhance(
            &scope(&request.context, "first"),
            az_plugin_contract::EnhancementRequest {
                path: "/v1/optional/missing".into(),
                input: serde_json::Value::Null,
            },
        )
        .await
        .err()
        .context("未装配增强服务却接受了调用")?;
    assert!(error.to_string().contains("增强服务未绑定"));
    assert!(
        !services
            .authorize(&scope(&request.context, "second"), "read")
            .await?
    );
    assert!(
        services
            .delegate(&request.context, "second", "third")
            .is_err()
    );
    let delegated = services.delegate(&request.context, "first", "second")?;
    assert!(
        services
            .management_session(&scope(&delegated.context, "second"))
            .is_err()
    );
    assert_ne!(delegated.context.request_id, request.context.request_id);
    assert_eq!(delegated.context.user_id, request.context.user_id);
    assert!(
        services
            .authorize(&scope(&delegated.context, "second"), "read")
            .await?
    );
    assert!(
        !services
            .authorize(&scope(&delegated.context, "first"), "read")
            .await?
    );
    let third = services.delegate(&delegated.context, "second", "third")?;
    drop(request);
    assert!(
        services
            .interactive(&delegated.context.request_id, "tenant", "second")
            .is_err()
    );
    assert!(
        services
            .interactive(&third.context.request_id, "tenant", "third")
            .is_err()
    );
    assert!(
        !services
            .authorize(&scope(&delegated.context, "second"), "read")
            .await?
    );
    assert!(
        services
            .delegate(&delegated.context, "second", "fourth")
            .is_err()
    );
    drop(delegated);
    drop(third);
    assert!(
        services
            .requests
            .lock()
            .map_err(|_| anyhow::anyhow!("测试请求锁损坏"))?
            .is_empty()
    );
    Ok(())
}

#[tokio::test]
async fn background_authority_is_bound_to_target_without_becoming_interactive() -> Result<()> {
    let services = Arc::new(Services::default());
    let request = services.background(
        "tenant",
        "user",
        "first",
        "second",
        vec![permission("second", "read")],
    )?;
    assert_eq!(request.context.session_id.as_deref(), Some("service:first"));
    assert!(
        services
            .management_session(&scope(&request.context, "second"))
            .is_err()
    );
    assert!(
        services
            .authorize(&scope(&request.context, "second"), "read")
            .await?
    );
    assert!(
        !services
            .authorize(&scope(&request.context, "first"), "read")
            .await?
    );
    assert!(
        services
            .interactive(&request.context.request_id, "tenant", "second")
            .is_err()
    );
    let delegated = services.delegate(&request.context, "second", "third")?;
    assert!(
        services
            .interactive(&delegated.context.request_id, "tenant", "third")
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn permissions_cannot_cross_plugin_or_host_boundaries() -> Result<()> {
    let services = Arc::new(Services::default());
    let mut session = session();
    session.permissions = vec![
        permission("first", "plugin:manage"),
        "host:read".into(),
        permission("second", "plugin:manage"),
    ];
    let request = services.enter(&session, "first")?;
    let mut scope = scope(&request.context, "first");
    assert!(services.authorize(&scope, "plugin:manage").await?);
    assert!(!services.authorize(&scope, "host:read").await?);
    scope.source_id = "second".into();
    assert!(!services.authorize(&scope, "plugin:manage").await?);
    scope.source_id = "first".into();
    scope.context.user_id = Some("forged".into());
    assert!(!services.authorize(&scope, "plugin:manage").await?);
    let request_id = request.context.request_id.clone();
    drop(request);
    assert!(
        services
            .interactive(&request_id, "tenant", "first")
            .is_err()
    );
    Ok(())
}
