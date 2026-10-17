use crate::runtime::{PageDefinition, PluginRuntime, PublishState};

/// 当前活动进程的宿主句柄。
pub struct ProcessBinding {
    pub instance_id: String,
}

/// 租户选定的版本及其产物，用于恢复与回滚。
pub struct BoundRuntime {
    pub revision_id: String,
    pub revision: String,
    pub runtime: PluginRuntime,
    pub artifact: String,
}

/// 活动服务的运行目标、地址和已声明路由。
pub struct ServiceBinding {
    pub runtime: PluginRuntime,
    pub revision: String,
    pub endpoint: Option<String>,
    pub routes: Vec<String>,
}

/// 页面与安装代数、状态代数绑定，防止旧页面继续写入。
pub struct PageBinding {
    pub source_id: String,
    pub activation_generation: String,
    pub revision_id: String,
    pub state_generation: Option<i64>,
    pub page: PageDefinition,
    pub service: ServiceBinding,
}

/// 宿主启动时需要恢复的进程安装。
pub struct ProcessTarget {
    pub tenant_id: String,
    pub source_id: String,
    pub revision_id: String,
    pub revision: String,
}

/// 宿主启动时需要恢复的 Wasm 安装。
pub struct WasmTarget {
    pub tenant_id: String,
    pub source_id: String,
    pub revision_id: String,
    pub revision: String,
    pub artifact: String,
}

/// 持久发布任务，由管理插件恢复和执行。
#[derive(Clone)]
pub struct PublishJob {
    pub id: String,
    pub tenant_id: String,
    pub source_id: String,
    pub git: String,
    pub revision: String,
    pub runtime: PluginRuntime,
    pub page_count: usize,
    pub state: PublishState,
    pub detail: String,
}

/// 发布凭据的租户与来源绑定。
pub struct PublisherBinding {
    pub tenant_id: String,
    pub git: String,
}
