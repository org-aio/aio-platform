use super::{
    model::ViewOwner,
    service::{Peer, ViewChannel, WorkerWebviewService},
    util,
};
use crate::{
    generated::worker::model::{DeviceIdentity, Worker},
    runtime::server::http_error::RuntimeError,
};
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use std::{collections::HashMap, sync::Mutex, time::Duration};
use tokio::sync::{mpsc, oneshot};

pub(crate) struct WorkerWebviewServiceImpl {
    pool: PgPool,
    lifecycle: tokio::sync::Mutex<()>,
    peers: Mutex<HashMap<String, Peer>>,
    views: Mutex<HashMap<String, ViewChannel>>,
}

#[dill::component(pub(crate))]
#[dill::interface(dyn WorkerWebviewService)]
#[dill::scope(dill::Singleton)]
impl WorkerWebviewServiceImpl {
    fn new(pool: PgPool) -> Self {
        Self {
            pool,
            lifecycle: tokio::sync::Mutex::new(()),
            peers: Mutex::default(),
            views: Mutex::default(),
        }
    }
}

#[async_trait::async_trait]
impl WorkerWebviewService for WorkerWebviewServiceImpl {
    async fn access(&self, device: &DeviceIdentity, enabled: bool) -> Result<(), RuntimeError> {
        let _guard = self.lifecycle.lock().await;
        sqlx::query("UPDATE worker_devices SET capabilities=CASE WHEN $2 THEN CASE WHEN capabilities ? 'codex.web' THEN capabilities ELSE capabilities || '[\"codex.web\"]'::jsonb END ELSE capabilities-'codex.web' END WHERE id=$1 AND state='active'")
            .bind(&device.id).bind(enabled).execute(&self.pool).await?;
        if !enabled {
            self.disconnect(&device.id)?;
            sqlx::query("UPDATE worker_webview_sessions SET state='closed' WHERE worker_id=$1")
                .bind(&device.id)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    async fn devices(&self, owner: &ViewOwner) -> Result<Vec<Worker>, RuntimeError> {
        let rows = sqlx::query("SELECT id,label,platform,capabilities FROM worker_devices WHERE tenant_id=$1 AND user_id=$2 AND state='active' AND capabilities ? 'codex.web' ORDER BY created_at DESC LIMIT 100")
            .bind(&owner.tenant).bind(&owner.user).fetch_all(&self.pool).await?;
        let peers = self
            .peers
            .lock()
            .map_err(|_| RuntimeError::unavailable("设备连接状态不可用"))?;
        rows.into_iter()
            .map(|row| {
                let id: String = row.try_get("id")?;
                let online = peers.contains_key(&id);
                Ok(Worker {
                    id,
                    label: row.try_get("label")?,
                    platform: row.try_get("platform")?,
                    capabilities: serde_json::from_value(row.try_get("capabilities")?)?,
                    status: if online { "online" } else { "offline" }.into(),
                    last_seen: None,
                })
            })
            .collect()
    }

    async fn create(
        &self,
        owner: &ViewOwner,
        device: &str,
        route: &str,
    ) -> Result<String, RuntimeError> {
        let _guard = self.lifecycle.lock().await;
        util::route(route)?;
        uuid::Uuid::parse_str(device)?;
        let active: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM worker_devices WHERE id=$1 AND tenant_id=$2 AND user_id=$3 AND state='active' AND capabilities ? 'codex.web')")
            .bind(device).bind(&owner.tenant).bind(&owner.user).fetch_one(&self.pool).await?;
        if !active
            || !self
                .peers
                .lock()
                .map_err(|_| RuntimeError::unavailable("连接状态不可用"))?
                .contains_key(device)
        {
            return Err(RuntimeError::forbidden("设备不在线或未启用 Codex 网页访问"));
        }
        let mut tx = self.pool.begin().await?;
        // 同一挂载最多保留一个视图，重连从原生事实恢复，不重放旧请求。
        sqlx::query("SELECT id FROM worker_devices WHERE id=$1 FOR UPDATE")
            .bind(device)
            .fetch_one(&mut *tx)
            .await?;
        let old: Vec<String> = sqlx::query_scalar("UPDATE worker_webview_sessions SET state='closed' WHERE mount_digest=$1 AND state!='closed' RETURNING id")
            .bind(util::digest(&owner.mount)).fetch_all(&mut *tx).await?;
        sqlx::query(
            "DELETE FROM worker_webview_sessions WHERE expires_at < now() - interval '1 day'",
        )
        .execute(&mut *tx)
        .await?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM worker_webview_sessions WHERE worker_id=$1 AND state!='closed' AND expires_at>now()")
            .bind(device).fetch_one(&mut *tx).await?;
        if count >= 4 {
            return Err(RuntimeError::conflict("该设备的网页连接已满"));
        }
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO worker_webview_sessions(id,worker_id,tenant_id,user_id,session_id,source_id,revision,mount_digest) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
            .bind(&id).bind(device).bind(&owner.tenant).bind(&owner.user).bind(&owner.session).bind(&owner.source).bind(&owner.revision).bind(util::digest(&owner.mount)).execute(&mut *tx).await?;
        tx.commit().await?;
        for id in old {
            self.close_channel(&id)?;
        }
        let (sender, receiver) = mpsc::channel(32);
        self.views
            .lock()
            .map_err(|_| RuntimeError::unavailable("视图状态不可用"))?
            .insert(
                id.clone(),
                ViewChannel {
                    worker: device.into(),
                    sender,
                    receiver: Some(receiver),
                    assets: HashMap::new(),
                },
            );
        if let Err(error) = self.send(device, json!({"kind":"open","sessionId":id,"route":route})) {
            self.close(owner, &id).await?;
            return Err(error);
        }
        Ok(id)
    }

    async fn authorize(&self, owner: &ViewOwner, id: &str) -> Result<String, RuntimeError> {
        uuid::Uuid::parse_str(id)?;
        sqlx::query_scalar("SELECT v.worker_id FROM worker_webview_sessions v JOIN worker_devices w ON w.id=v.worker_id WHERE v.id=$1 AND v.tenant_id=$2 AND v.user_id=$3 AND v.session_id=$4 AND v.source_id=$5 AND v.revision=$6 AND v.mount_digest=$7 AND v.state!='closed' AND v.expires_at>now() AND w.state='active' AND w.tenant_id=v.tenant_id AND w.user_id=v.user_id AND w.capabilities ? 'codex.web'")
            .bind(id).bind(&owner.tenant).bind(&owner.user).bind(&owner.session).bind(&owner.source).bind(&owner.revision).bind(util::digest(&owner.mount))
            .fetch_optional(&self.pool).await?.ok_or_else(|| RuntimeError::forbidden("网页连接已过期或设备授权已撤销"))
    }

    async fn register(
        &self,
        device: &DeviceIdentity,
    ) -> Result<(String, mpsc::Receiver<Value>), RuntimeError> {
        let _guard = self.lifecycle.lock().await;
        let enabled: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM worker_devices WHERE id=$1 AND state='active' AND capabilities ? 'codex.web')").bind(&device.id).fetch_one(&self.pool).await?;
        if !enabled {
            return Err(RuntimeError::forbidden("设备未启用 Codex 网页访问"));
        }
        self.disconnect(&device.id)?;
        // 设备重连意味着旧 Native 端口已丢失，全部旧视图必须重新建立。
        sqlx::query("UPDATE worker_webview_sessions SET state='closed' WHERE worker_id=$1 AND state!='closed'").bind(&device.id).execute(&self.pool).await?;
        let generation = uuid::Uuid::new_v4().to_string();
        let (sender, receiver) = mpsc::channel(32);
        let mut peers = self
            .peers
            .lock()
            .map_err(|_| RuntimeError::unavailable("连接状态不可用"))?;
        if peers.len() >= 64 {
            return Err(RuntimeError::unavailable("设备通道已满"));
        }
        peers.insert(
            device.id.clone(),
            Peer {
                generation: generation.clone(),
                sender,
            },
        );
        Ok((generation, receiver))
    }

    async fn expire(&self, device: &str) -> Result<(), RuntimeError> {
        let ids: Vec<String> = sqlx::query_scalar("UPDATE worker_webview_sessions SET state='closed' WHERE worker_id=$1 AND (expires_at<=now() OR state='closed' OR (state='waiting' AND created_at<now()-interval '90 seconds')) RETURNING id")
            .bind(device).fetch_all(&self.pool).await?;
        for id in ids {
            self.close_channel(&id)?;
        }
        Ok(())
    }

    async fn unregister(&self, device: &str, generation: &str) {
        let _guard = self.lifecycle.lock().await;
        let current = self
            .peers
            .lock()
            .map(|peers| {
                peers
                    .get(device)
                    .is_some_and(|peer| peer.generation == generation)
            })
            .unwrap_or(false);
        if current {
            let _ = self.disconnect(device);
            let _ =
                sqlx::query("UPDATE worker_webview_sessions SET state='closed' WHERE worker_id=$1")
                    .bind(device)
                    .execute(&self.pool)
                    .await;
        }
    }

    async fn attach(
        &self,
        owner: &ViewOwner,
        id: &str,
    ) -> Result<mpsc::Receiver<Value>, RuntimeError> {
        self.authorize(owner, id).await?;
        let claimed = sqlx::query(
            "UPDATE worker_webview_sessions SET state='active' WHERE id=$1 AND state='waiting'",
        )
        .bind(id)
        .execute(&self.pool)
        .await?
        .rows_affected();
        if claimed != 1 {
            return Err(RuntimeError::conflict("该视图已连接，请重新连接设备"));
        }
        self.views
            .lock()
            .map_err(|_| RuntimeError::unavailable("视图状态不可用"))?
            .get_mut(id)
            .and_then(|view| view.receiver.take())
            .ok_or_else(|| RuntimeError::conflict("视图通道已关闭"))
    }

    async fn frame(&self, owner: &ViewOwner, id: &str, frame: Value) -> Result<(), RuntimeError> {
        let worker = self.authorize(owner, id).await?;
        // 宿主不解释 Harness 消息；固定帧结构和原生方法白名单由设备校验。
        if !frame.is_object() || serde_json::to_vec(&frame)?.len() > 16 * 1024 * 1024 {
            return Err(RuntimeError::bad_request("网页消息无效或过大"));
        }
        self.send(
            &worker,
            json!({"kind":"frame","sessionId":id,"frame":frame}),
        )
    }

    async fn asset(&self, owner: &ViewOwner, id: &str, path: &str) -> Result<Value, RuntimeError> {
        util::asset_path(path)?;
        let worker = self.authorize(owner, id).await?;
        let request = uuid::Uuid::new_v4().to_string();
        let (send, receive) = oneshot::channel();
        {
            let mut views = self
                .views
                .lock()
                .map_err(|_| RuntimeError::unavailable("视图状态不可用"))?;
            let view = views
                .get_mut(id)
                .ok_or_else(|| RuntimeError::conflict("视图尚未连接"))?;
            view.assets.retain(|_, waiter| !waiter.is_closed());
            if view.assets.len() >= 32 {
                return Err(RuntimeError::unavailable("资源请求已满"));
            }
            view.assets.insert(request.clone(), send);
        }
        let result = async {
            self.send(
                &worker,
                json!({"kind":"asset","sessionId":id,"id":request,"path":path}),
            )?;
            tokio::time::timeout(Duration::from_secs(30), receive)
                .await
                .map_err(|_| RuntimeError::unavailable("设备资源请求超时"))?
                .map_err(|_| RuntimeError::unavailable("设备资源连接已关闭"))
        }
        .await;
        if let Ok(mut views) = self.views.lock()
            && let Some(view) = views.get_mut(id)
        {
            view.assets.remove(&request);
        }
        result
    }

    async fn receive(
        &self,
        device: &str,
        generation: &str,
        frame: Value,
    ) -> Result<(), RuntimeError> {
        let current = self
            .peers
            .lock()
            .map_err(|_| RuntimeError::unavailable("连接状态不可用"))?
            .get(device)
            .is_some_and(|peer| peer.generation == generation);
        if !current {
            return Err(RuntimeError::forbidden("设备通道已替换"));
        }
        let id = frame
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or_else(|| RuntimeError::bad_request("视图 ID 缺失"))?
            .to_owned();
        let sender = {
            let mut views = self
                .views
                .lock()
                .map_err(|_| RuntimeError::unavailable("视图状态不可用"))?;
            let Some(view) = views.get_mut(&id) else {
                return Ok(());
            };
            if view.worker != device {
                return Err(RuntimeError::forbidden("视图不属于此设备"));
            }
            if frame.get("kind").and_then(Value::as_str) == Some("asset") {
                let request = frame
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| RuntimeError::bad_request("资源请求 ID 缺失"))?;
                if let Some(waiter) = view.assets.remove(request) {
                    let _ = waiter.send(frame);
                }
                return Ok(());
            }
            view.sender.clone()
        };
        // 初始化消息可能超过队列容量；在锁外按序等待，保留有界内存与心跳余量。
        let delivered = tokio::time::timeout(Duration::from_secs(10), sender.send(frame)).await;
        if delivered.is_ok_and(|result| result.is_ok()) {
            return Ok(());
        }
        // 网页关闭或持续阻塞只结束该视图，不能连带断开设备及其余资源请求。
        sqlx::query(
            "UPDATE worker_webview_sessions SET state='closed' WHERE id=$1 AND worker_id=$2",
        )
        .bind(&id)
        .bind(device)
        .execute(&self.pool)
        .await?;
        self.close_channel(&id)
    }

    async fn close(&self, owner: &ViewOwner, id: &str) -> Result<(), RuntimeError> {
        // 关闭操作也验证归属，允许重复关闭本人的连接。
        let worker: Option<String> = sqlx::query_scalar("UPDATE worker_webview_sessions SET state='closed' WHERE id=$1 AND tenant_id=$2 AND user_id=$3 AND mount_digest=$4 AND session_id=$5 AND source_id=$6 AND revision=$7 RETURNING worker_id")
            .bind(id).bind(&owner.tenant).bind(&owner.user).bind(util::digest(&owner.mount)).bind(&owner.session).bind(&owner.source).bind(&owner.revision).fetch_optional(&self.pool).await?;
        let worker = worker.ok_or_else(|| RuntimeError::forbidden("视图不属于此挂载"))?;
        let _ = self.send(&worker, json!({"kind":"close","sessionId":id}));
        self.close_channel(id)
    }
}

impl WorkerWebviewServiceImpl {
    fn send(&self, device: &str, frame: Value) -> Result<(), RuntimeError> {
        let peers = self
            .peers
            .lock()
            .map_err(|_| RuntimeError::unavailable("设备状态不可用"))?;
        let peer = peers
            .get(device)
            .ok_or_else(|| RuntimeError::unavailable("设备已经断开"))?;
        peer.sender
            .try_send(frame)
            .map_err(|_| RuntimeError::unavailable("设备通道过载或已关闭"))
    }

    fn close_channel(&self, id: &str) -> Result<(), RuntimeError> {
        if let Some(view) = self
            .views
            .lock()
            .map_err(|_| RuntimeError::unavailable("视图状态不可用"))?
            .remove(id)
        {
            let _ = view
                .sender
                .try_send(json!({"kind":"closed","sessionId":id}));
            let _ = self.send(&view.worker, json!({"kind":"close","sessionId":id}));
        }
        Ok(())
    }

    fn disconnect(&self, device: &str) -> Result<(), RuntimeError> {
        self.peers
            .lock()
            .map_err(|_| RuntimeError::unavailable("设备状态不可用"))?
            .remove(device);
        let ids: Vec<String> = self
            .views
            .lock()
            .map_err(|_| RuntimeError::unavailable("视图状态不可用"))?
            .iter()
            .filter(|(_, v)| v.worker == device)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.close_channel(&id)?;
        }
        Ok(())
    }
}
