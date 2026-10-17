//! 默认业务服务拥有自己的持久化初始化，不依赖旧版插件目录。

use std::any::TypeId;

use anyhow::Context;
use az_plugin_core::{Plugin, PluginFuture};
use sqlx::PgPool;

use super::{DatabasePlugin, KeyringPlugin, RuntimeAssembly};
use crate::generated::{clipboard, personal_config, worker, worker_webview};

// 初始化与服务绑定由同一插件拥有；只迁移本次装配选中的能力。
macro_rules! postgres_service_plugin {
    ($name:ident, $implementation:ty, $interface:ty, [$($dependency:ty),*], $schema:literal) => {
        /// 注册默认 PostgreSQL 服务并初始化其业务表；停用保留持久记录。
        #[dill::component]
        #[dill::interface(dyn Plugin<RuntimeAssembly>)]
        pub struct $name;

        impl Plugin<RuntimeAssembly> for $name {
            fn dependencies(&self) -> Vec<TypeId> {
                vec![$(TypeId::of::<$dependency>()),*]
            }

            fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
                Box::pin(async move {
                    assembly.services.add::<$implementation>();
                    assembly.require_service::<PgPool>();
                    assembly.require_service::<$interface>();
                    assembly.on_initialize::<Self>(|state, _| {
                        Box::pin(async move {
                            let pool = state.service::<PgPool>()?;
                            let mut transaction = pool.begin().await?;
                            sqlx::Executor::execute(
                                &mut *transaction,
                                sqlx::raw_sql(include_str!($schema)),
                            )
                                .await
                                .context("初始化业务插件持久化结构失败")?;
                            transaction.commit().await?;
                            Ok(())
                        })
                    });
                    Ok(())
                })
            }
        }
    };
}

postgres_service_plugin!(
    WorkerServicePlugin,
    worker::WorkerServiceImpl,
    dyn worker::WorkerService,
    [DatabasePlugin],
    "../../../generated/worker/schema.sql"
);

// 这三个默认实现直接查询设备表；依赖其 PostgreSQL 提供者，而非仅依赖接口。
postgres_service_plugin!(
    WorkerWebviewServicePlugin,
    worker_webview::WorkerWebviewServiceImpl,
    dyn worker_webview::WorkerWebviewService,
    [WorkerServicePlugin],
    "../../../generated/worker_webview/schema.sql"
);

postgres_service_plugin!(
    PersonalConfigServicePlugin,
    personal_config::PersonalConfigServiceImpl,
    dyn personal_config::PersonalConfigService,
    [WorkerServicePlugin, KeyringPlugin],
    "../../../generated/personal_config/schema.sql"
);

postgres_service_plugin!(
    ClipboardServicePlugin,
    clipboard::ClipboardServiceImpl,
    dyn clipboard::ClipboardService,
    [WorkerServicePlugin, KeyringPlugin],
    "../../../generated/clipboard/schema.sql"
);
