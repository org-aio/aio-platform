use super::{WorkerWebviewService, WorkerWebviewServiceImpl, model::ViewOwner};
use crate::generated::worker::model::DeviceIdentity;
use anyhow::Result;
use serde_json::json;
use std::time::Duration;

/// 用真实数据库验证隔离、消息顺序、资源应答、单一连接和撤权。
#[tokio::test]
#[ignore = "需要隔离 PostgreSQL，设置 AIO_TEST_DATABASE_URL"]
async fn webview_channel_roundtrip_isolation_and_revocation() -> Result<()> {
    let database = std::env::var("AIO_TEST_DATABASE_URL")?;
    let admin = sqlx::PgPool::connect(&database).await?;
    let schema = format!("webview_test_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await?;
    let mut url = reqwest::Url::parse(&database)?;
    url.query_pairs_mut()
        .append_pair("options", &format!("-c search_path={schema}"));
    let pool = sqlx::PgPool::connect(url.as_str()).await?;
    sqlx::raw_sql(include_str!("../worker/schema.sql"))
        .execute(&pool)
        .await?;
    sqlx::raw_sql(include_str!("schema.sql"))
        .execute(&pool)
        .await?;
    let service = dill::Catalog::builder()
        .add_value(pool.clone())
        .add::<WorkerWebviewServiceImpl>()
        .build()
        .get_one::<dyn WorkerWebviewService>()?;
    let device = DeviceIdentity {
        id: uuid::Uuid::new_v4().to_string(),
        tenant: "tenant".into(),
        user: "user".into(),
        capabilities: vec!["codex.web".into()],
    };
    sqlx::query("INSERT INTO worker_devices(id,token_hash,label,platform,tenant_id,user_id,state,capabilities) VALUES($1,$2,'fixture','darwin','tenant','user','active','[\"codex.web\"]')")
        .bind(&device.id).bind(uuid::Uuid::new_v4().to_string()).execute(&pool).await?;
    let owner = ViewOwner {
        tenant: device.tenant.clone(),
        user: device.user.clone(),
        session: "login-session".into(),
        source: "plugin".into(),
        revision: "revision1".into(),
        mount: "mount1".into(),
    };
    let (generation, mut commands) = service
        .register(&device)
        .await
        .map_err(|_| anyhow::anyhow!("注册失败"))?;
    let id = service
        .create(&owner, &device.id, "/")
        .await
        .map_err(|_| anyhow::anyhow!("创建失败"))?;
    assert_eq!(
        commands.recv().await,
        Some(json!({"kind":"open","sessionId":id,"route":"/"}))
    );
    for different in [
        ViewOwner {
            tenant: "other".into(),
            ..owner.clone()
        },
        ViewOwner {
            user: "other".into(),
            ..owner.clone()
        },
        ViewOwner {
            session: "other".into(),
            ..owner.clone()
        },
        ViewOwner {
            source: "other".into(),
            ..owner.clone()
        },
        ViewOwner {
            revision: "other".into(),
            ..owner.clone()
        },
        ViewOwner {
            mount: "other".into(),
            ..owner.clone()
        },
    ] {
        assert!(service.authorize(&different, &id).await.is_err());
        assert!(service.close(&different, &id).await.is_err());
        assert!(service.authorize(&owner, &id).await.is_ok());
    }
    let mut browser = service
        .attach(&owner, &id)
        .await
        .map_err(|_| anyhow::anyhow!("连接失败"))?;
    assert!(service.attach(&owner, &id).await.is_err());
    for index in 0..3 {
        service
            .frame(&owner, &id, json!({"kind":"call","index":index}))
            .await
            .map_err(|_| anyhow::anyhow!("发送失败"))?;
    }
    for index in 0..3 {
        assert_eq!(
            commands
                .recv()
                .await
                .and_then(|frame| frame.get("frame").cloned()),
            Some(json!({"kind":"call","index":index}))
        );
    }
    service
        .receive(
            &device.id,
            &generation,
            json!({"kind":"frame","sessionId":id,"frame":{"kind":"native-message","payload":[]}}),
        )
        .await
        .map_err(|_| anyhow::anyhow!("接收失败"))?;
    assert_eq!(
        browser
            .recv()
            .await
            .and_then(|frame| frame.get("kind").cloned()),
        Some(json!("frame"))
    );
    // 原版 Renderer 初始化会连续发送超过队列容量的消息，必须完整、按序交付。
    let burst_service = service.clone();
    let burst_device = device.id.clone();
    let burst_generation = generation.clone();
    let burst_id = id.clone();
    let burst = tokio::spawn(async move {
        for index in 0..96 {
            burst_service
                .receive(
                    &burst_device,
                    &burst_generation,
                    json!({"kind":"frame","sessionId":burst_id,"index":index}),
                )
                .await
                .map_err(|_| anyhow::anyhow!("初始化消息 {index} 未交付"))?;
        }
        anyhow::Ok(())
    });
    for index in 0..96 {
        let frame = tokio::time::timeout(Duration::from_secs(2), browser.recv())
            .await?
            .ok_or_else(|| anyhow::anyhow!("初始化通道提前关闭"))?;
        assert_eq!(frame["index"], index);
    }
    burst.await??;

    // 一个网页关闭后，设备通道仍应服务其他挂载和资源请求。
    let other_owner = ViewOwner {
        mount: "mount2".into(),
        ..owner.clone()
    };
    let other_id = service
        .create(&other_owner, &device.id, "/")
        .await
        .map_err(|_| anyhow::anyhow!("第二个视图创建失败"))?;
    assert_eq!(
        commands.recv().await,
        Some(json!({"kind":"open","sessionId":other_id,"route":"/"}))
    );
    let other_browser = service
        .attach(&other_owner, &other_id)
        .await
        .map_err(|_| anyhow::anyhow!("第二个视图连接失败"))?;
    drop(other_browser);
    service
        .receive(
            &device.id,
            &generation,
            json!({"kind":"frame","sessionId":other_id,"frame":{"kind":"native-message","payload":[]}}),
        )
        .await
        .map_err(|_| anyhow::anyhow!("单一网页关闭中断了设备通道"))?;
    assert!(service.authorize(&other_owner, &other_id).await.is_err());
    assert!(service.authorize(&owner, &id).await.is_ok());
    assert_eq!(
        commands.recv().await,
        Some(json!({"kind":"close","sessionId":other_id}))
    );
    let copy = service.clone();
    let request_owner = owner.clone();
    let request_id = id.clone();
    let waiting = tokio::spawn(async move {
        copy.asset(&request_owner, &request_id, "assets/main.js")
            .await
            .map_err(|_| anyhow::anyhow!("资源失败"))
    });
    let asset_command = tokio::time::timeout(Duration::from_secs(2), commands.recv())
        .await?
        .ok_or_else(|| anyhow::anyhow!("资源请求缺失"))?;
    let reply = json!({"kind":"asset","sessionId":id,"id":asset_command["id"],"data":"YQ==","contentType":"text/javascript"});
    service
        .receive(&device.id, &generation, reply.clone())
        .await
        .map_err(|_| anyhow::anyhow!("资源应答失败"))?;
    assert_eq!(waiting.await??, reply);
    service
        .access(&device, false)
        .await
        .map_err(|_| anyhow::anyhow!("撤权失败"))?;
    assert!(service.authorize(&owner, &id).await.is_err());
    assert!(
        service
            .frame(&owner, &id, json!({"kind":"connect"}))
            .await
            .is_err()
    );
    assert!(service.register(&device).await.is_err());
    assert_eq!(
        browser
            .recv()
            .await
            .and_then(|frame| frame.get("kind").cloned()),
        Some(json!("closed"))
    );
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await?;
    admin.close().await;
    Ok(())
}
