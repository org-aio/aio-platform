//! 匿名资源和交互下载共用字节预算、背压、截止时间与持续授权检查。
use std::{sync::Arc, time::Duration};

use super::super::plugins::PluginTasks;
use anyhow::{Context, Result, ensure};
use axum::{body::Body, http::HeaderMap};
use futures_util::{StreamExt, stream::BoxStream};
use tokio::{
    sync::{OwnedSemaphorePermit, mpsc, oneshot},
    time::Instant,
};
use uuid::Uuid;

use super::super::{RuntimeState, request_context::authenticate};
use super::{Components, services::Authorization};

pub(super) enum Access {
    Public {
        components: Arc<Components>,
        source: Uuid,
        tenant: String,
        digest: String,
        generation: String,
    },
    Session {
        state: RuntimeState,
        headers: HeaderMap,
        token: String,
        _authorization: Box<Authorization>,
    },
}

impl Access {
    pub async fn check(&self) -> Result<()> {
        match self {
            Self::Public {
                components,
                source,
                tenant,
                digest,
                generation,
            } => {
                ensure!(
                    components
                        .public_generation(*source, tenant, digest)
                        .await?
                        == *generation,
                    "公开资源安装代数已变化"
                );
            }
            Self::Session {
                state,
                headers,
                token,
                ..
            } => {
                let session = authenticate(state, headers)
                    .await
                    .map_err(|_| anyhow::anyhow!("下载会话已撤销"))?;
                super::frontend::validate(state, &session, token)
                    .await
                    .map_err(|_| anyhow::anyhow!("下载权限或插件版本已变化"))?;
            }
        }
        Ok(())
    }
}

pub(super) struct Transfer {
    pub access: Access,
    pub tasks: Arc<PluginTasks>,
    pub limit: u64,
    pub body: BoxStream<'static, Result<Vec<u8>>>,
    pub bytes: u64,
    pub expected: Option<u64>,
    pub deadline: Instant,
    pub next_check: Instant,
    pub _permit: OwnedSemaphorePermit,
}

impl Transfer {
    pub fn body(mut self) -> Result<Body> {
        let (sender, receiver) = mpsc::channel(1);
        let (finished, completion) = oneshot::channel();
        let tasks = self.tasks.clone();
        let cancellation = tasks.cancellation();
        tasks.spawn(async move {
            // 慢速客户端不再轮询 Body 时，截止时间和插件取消仍能释放上游与并发槽。
            let result = tokio::select! {
                biased;
                _ = cancellation.cancelled() => Err(anyhow::anyhow!("资源传输所属插件已停止")),
                _ = sender.closed() => Ok(()),
                result = self.send(&sender) => result,
            };
            let _ = finished.send(result);
            Ok(())
        })?;
        let stream = futures_util::stream::try_unfold(
            (receiver, Some(completion)),
            |(mut receiver, mut completion)| async move {
                if let Some(bytes) = receiver.recv().await {
                    return Ok(Some((bytes, (receiver, completion))));
                }
                if let Some(completion) = completion.take() {
                    completion.await.context("资源传输任务中断")??;
                }
                Ok::<_, anyhow::Error>(None)
            },
        );
        Ok(Body::from_stream(stream))
    }

    async fn send(&mut self, sender: &mpsc::Sender<Vec<u8>>) -> Result<()> {
        loop {
            self.check().await?;
            let item = tokio::select! {
                item = self.body.next() => item,
                _ = tokio::time::sleep_until(self.next_check.min(self.deadline)) => continue,
            };
            self.check().await?;
            let Some(bytes) = item else {
                ensure!(
                    self.expected.is_none_or(|expected| expected == self.bytes),
                    "资源响应长度不符"
                );
                return Ok(());
            };
            let bytes = bytes?;
            if bytes.is_empty() {
                tokio::task::yield_now().await;
                continue;
            }
            self.bytes = self
                .bytes
                .checked_add(bytes.len() as u64)
                .context("资源响应大小溢出")?;
            ensure!(
                self.bytes <= self.limit && self.expected.is_none_or(|length| self.bytes <= length),
                "资源响应超过配额"
            );
            // 等待下游信用时仍检查撤权，并将队列中的单帧限制在 64 KiB。
            for chunk in bytes.chunks(64 * 1024) {
                loop {
                    self.check().await?;
                    tokio::select! {
                        slot = sender.reserve() => {
                            slot.context("下载客户端已关闭")?.send(chunk.to_vec());
                            break;
                        },
                        _ = tokio::time::sleep_until(self.next_check.min(self.deadline)) => {},
                    }
                }
            }
        }
    }

    async fn check(&mut self) -> Result<()> {
        ensure!(Instant::now() < self.deadline, "资源传输超时");
        if Instant::now() >= self.next_check {
            tokio::time::timeout_at(self.deadline, self.access.check()).await??;
            self.next_check = Instant::now() + Duration::from_millis(500);
        }
        Ok(())
    }
}
