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
    assert!(service.renew(&owner, &id).await.is_err());
    let mut browser = service
        .attach(&owner, &id)
        .await
        .map_err(|_| anyhow::anyhow!("连接失败"))?;
    assert!(service.attach(&owner, &id).await.is_err());
    sqlx::query(
        "UPDATE worker_webview_sessions SET expires_at=now()+interval '1 minute' WHERE id=$1",
    )
    .bind(&id)
    .execute(&pool)
    .await?;
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
        assert!(service.renew(&different, &id).await.is_err());
        assert!(service.close(&different, &id).await.is_err());
        assert!(service.authorize(&owner, &id).await.is_ok());
    }
    // 普通授权和错误归属都不能续期；合法存活心跳才能延长即将过期的连接。
    let unchanged: bool = sqlx::query_scalar(
        "SELECT expires_at<=now()+interval '1 minute' FROM worker_webview_sessions WHERE id=$1",
    )
    .bind(&id)
    .fetch_one(&pool)
    .await?;
    assert!(unchanged);
    service
        .renew(&owner, &id)
        .await
        .map_err(|_| anyhow::anyhow!("续期失败"))?;
    let renewed: bool = sqlx::query_scalar(
        "SELECT expires_at>now()+interval '29 minutes' FROM worker_webview_sessions WHERE id=$1",
    )
    .bind(&id)
    .fetch_one(&pool)
    .await?;
    assert!(renewed);
    for update in [
        "UPDATE worker_devices SET state='revoked' WHERE id=$1",
        "UPDATE worker_devices SET state='active',capabilities='[]' WHERE id=$1",
    ] {
        sqlx::query(update).bind(&device.id).execute(&pool).await?;
        assert!(service.renew(&owner, &id).await.is_err());
    }
    sqlx::query("UPDATE worker_devices SET capabilities='[\"codex.web\"]' WHERE id=$1")
        .bind(&device.id)
        .execute(&pool)
        .await?;
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
            .and_then(|frame| frame.value.get("kind").cloned()),
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
        assert_eq!(frame.value["index"], index);
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
    // 第二个网页尚未加载完成时，初始化突发不能挡住原网页及同一设备的资源回包。
    for index in 0..96 {
        tokio::time::timeout(
            Duration::from_millis(250),
            service.receive(
                &device.id,
                &generation,
                json!({"kind":"frame","sessionId":other_id,"index":index}),
            ),
        )
        .await?
        .map_err(|_| anyhow::anyhow!("等待中的视图阻塞共享通道"))?;
    }
    service
        .receive(
            &device.id,
            &generation,
            json!({"kind":"frame","sessionId":id,"index":"still-active"}),
        )
        .await
        .map_err(|_| anyhow::anyhow!("原网页被第二个网页阻塞"))?;
    assert_eq!(browser.recv().await.unwrap().value["index"], "still-active");
    let mut other_browser = service
        .attach(&other_owner, &other_id)
        .await
        .map_err(|_| anyhow::anyhow!("第二个视图连接失败"))?;
    for index in 0..96 {
        assert_eq!(other_browser.recv().await.unwrap().value["index"], index);
    }
    // 超出独立帧数上限只关闭慢网页，原网页保持在线。
    for index in 0..257 {
        tokio::time::timeout(
            Duration::from_millis(250),
            service.receive(
                &device.id,
                &generation,
                json!({"kind":"frame","sessionId":other_id,"index":index}),
            ),
        )
        .await?
        .map_err(|_| anyhow::anyhow!("慢网页拖住了设备 reader"))?;
    }
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
    assert!(service.renew(&other_owner, &other_id).await.is_err());
    assert!(service.authorize(&owner, &id).await.is_ok());
    assert_eq!(
        commands.recv().await,
        Some(json!({"kind":"close","sessionId":other_id}))
    );
    // 少量大帧也受总字节预算约束，不能只依赖帧数上限。
    let large_id = service
        .create(&other_owner, &device.id, "/")
        .await
        .map_err(|_| anyhow::anyhow!("大帧测试视图创建失败"))?;
    assert_eq!(commands.recv().await.unwrap()["kind"], "open");
    for _ in 0..3 {
        service
            .receive(
                &device.id,
                &generation,
                json!({"kind":"frame","sessionId":large_id,"data":"x".repeat(12 * 1024 * 1024)}),
            )
            .await
            .map_err(|_| anyhow::anyhow!("大帧预算未隔离"))?;
    }
    assert!(service.authorize(&other_owner, &large_id).await.is_err());
    assert!(service.authorize(&owner, &id).await.is_ok());
    assert_eq!(
        commands.recv().await,
        Some(json!({"kind":"close","sessionId":large_id}))
    );
    // 即使原网页仍持有接收端，已经过期的视图也不能被心跳复活。
    let expired_id = service
        .create(&other_owner, &device.id, "/")
        .await
        .map_err(|_| anyhow::anyhow!("过期视图创建失败"))?;
    assert_eq!(
        commands.recv().await,
        Some(json!({"kind":"open","sessionId":expired_id,"route":"/"}))
    );
    let _expired_browser = service
        .attach(&other_owner, &expired_id)
        .await
        .map_err(|_| anyhow::anyhow!("过期视图连接失败"))?;
    sqlx::query(
        "UPDATE worker_webview_sessions SET expires_at=now()-interval '1 second' WHERE id=$1",
    )
    .bind(&expired_id)
    .execute(&pool)
    .await?;
    assert!(service.renew(&other_owner, &expired_id).await.is_err());
    assert!(service.authorize(&other_owner, &expired_id).await.is_err());
    service
        .expire(&device.id)
        .await
        .map_err(|_| anyhow::anyhow!("过期清理失败"))?;
    assert_eq!(
        commands.recv().await,
        Some(json!({"kind":"close","sessionId":expired_id}))
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
    assert!(service.renew(&owner, &id).await.is_err());
    assert!(
        service
            .frame(&owner, &id, json!({"kind":"connect"}))
            .await
            .is_err()
    );
    assert!(service.register(&device).await.is_err());
    assert!(browser.recv().await.is_none());
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await?;
    admin.close().await;
    Ok(())
}
