//! 整包执行器使用同一生命周期契约；安装提交与租户授权仍由组件管理器负责。

use std::{
    any::Any,
    collections::HashSet,
    panic::AssertUnwindSafe,
    path::PathBuf,
    sync::{Arc, Weak},
};

use anyhow::{Context, Result, ensure};
use az_plugin_bundle::{Bundle, VerifiedBundle};
use az_plugin_contract::RequestContext;
use az_plugin_development::{DevArtifact, DevLaunch};
use az_plugin_runtime::InvocationResources;
use futures_util::FutureExt;
use uuid::Uuid;

pub use super::model::{Description as ComponentDescription, Page as ComponentPage};
pub use az_plugin_contract::transport::{Request, Response};

/// 宿主交给原生执行器的资源入口；弱引用避免执行器和组件管理器相互持有。
/// 原生执行器属于可信宿主代码，脚本和进程插件不能获得此对象。
#[derive(Clone)]
pub struct ExecutionContext {
    /// 当前组件宿主的数据根目录，执行器应在其子目录内保存自身资源。
    pub root: PathBuf,
    pub(super) components: Weak<super::Components>,
}

impl ExecutionContext {
    pub(super) fn components(&self) -> Result<Arc<super::Components>> {
        self.components.upgrade().context("组件宿主已停止")
    }

    /// 按整包能力准备租户隔离的数据库、对象存储、密钥及宿主服务。
    pub async fn resources(
        &self,
        source: Uuid,
        tenant: &str,
        bundle: &VerifiedBundle,
    ) -> Result<InvocationResources> {
        self.components()?
            .resources_verified(source, tenant, bundle)
            .await
    }

    /// 原生实现需要其他插件服务时，从宿主同一个 Dill 容器解析。
    pub fn services(&self) -> Result<dill::Catalog> {
        Ok(self.components()?.runtime_services.upgrade())
    }
}

/// 可替换的整包执行器；具体类型的 TypeId 是原生插件身份，整包格式只用于匹配输入。
/// initialize 和 shutdown 必须允许部分初始化；关闭保留持久版本，deactivate 才停用实例。
/// activate 可能改变在线状态，失败时组件管理器会按安装记录调用 restore 补偿。
#[async_trait::async_trait]
pub trait ComponentExecutor: Any + Send + Sync {
    /// 无副作用地判断整包格式；相同配置和整包在整个宿主生命周期中结果必须稳定。
    fn supports(&self, bundle: &VerifiedBundle) -> bool;
    /// 在宿主资源就绪后初始化，失败或部分完成时仍必须允许 shutdown。
    async fn initialize(&self, context: ExecutionContext) -> Result<()>;
    /// 验证候选及页面描述，不改变任何已安装实例；返回前应用 with_settings。
    async fn validate(&self, source: Uuid, bundle: &Bundle) -> Result<ComponentDescription>;
    /// 准备并切换候选实例，安装事务由宿主在成功返回后提交。
    async fn activate(&self, source: Uuid, tenant: &str, bundle: &Bundle) -> Result<()>;
    /// 按给定的已提交整包修复执行状态，用于重启或失败补偿。
    async fn restore(&self, source: Uuid, tenant: &str, bundle: &Bundle) -> Result<()>;
    /// 返回该租户实例当前实际使用的整包，不能只读取发布目录。
    async fn bundle(&self, source: Uuid, tenant: &str) -> Result<Option<Arc<VerifiedBundle>>>;
    /// 执行已授权请求，仍须检查实例摘要、租户及实现自身的配额。
    async fn handle(
        &self,
        source: Uuid,
        tenant: &str,
        digest: &str,
        request: Request,
        context: RequestContext,
    ) -> Result<Response>;
    /// 释放指定实例；宿主随后提交停用或卸载记录。
    async fn deactivate(&self, source: Uuid, tenant: &str) -> Result<()>;
    /// 释放本次宿主拥有的全部资源，保留持久安装与活动版本。
    async fn shutdown(&self) -> Result<()>;

    /// 安装记录恢复完成后，拥有外部资源的实现可以清理孤立实例。
    async fn reconcile(&self) -> Result<()> {
        Ok(())
    }

    /// 需要 CLI 启动外部进程的执行器返回启动环境，其余实现明确拒绝该操作。
    async fn prepare_development(
        &self,
        _artifact: &DevArtifact,
        _bundle: Arc<VerifiedBundle>,
    ) -> Result<DevLaunch> {
        anyhow::bail!("此执行器不需要托管开发进程")
    }
    async fn activate_development(
        &self,
        _artifact: &DevArtifact,
        _bundle: Arc<VerifiedBundle>,
    ) -> Result<ComponentDescription> {
        anyhow::bail!("此执行器未提供开发装配")
    }
    async fn discard_development_candidate(&self) -> Result<()> {
        Ok(())
    }
}

/// 从 Dill 汇集原生执行器；不另建名称注册表，也不回退到未选择的内置实现。
#[dill::component]
#[dill::scope(dill::Singleton)]
pub struct ComponentExecutors {
    pub(super) implementations: Vec<Arc<dyn ComponentExecutor>>,
}

impl ComponentExecutors {
    /// 空集合保留组件管理面；宿主装配时只检查具体类型唯一性，不创建执行资源。
    pub fn validate(&self) -> Result<()> {
        let mut identities = HashSet::new();
        for implementation in &self.implementations {
            ensure!(
                identities.insert(implementation.as_ref().type_id()),
                "整包执行器类型被重复注册"
            );
        }
        Ok(())
    }

    pub(super) fn select(&self, bundle: &VerifiedBundle) -> Result<Arc<dyn ComponentExecutor>> {
        self.matching(bundle)?.context("未装配支持该整包的执行器")
    }

    pub(super) fn matching(
        &self,
        bundle: &VerifiedBundle,
    ) -> Result<Option<Arc<dyn ComponentExecutor>>> {
        let mut matches = self
            .implementations
            .iter()
            .filter(|executor| executor.supports(bundle));
        let executor = matches.next();
        ensure!(
            matches.next().is_none(),
            "同一整包存在多个执行器，必须明确选择实现"
        );
        Ok(executor.cloned())
    }

    pub(super) async fn initialize(&self, context: ExecutionContext) -> Result<()> {
        self.validate()?;
        for implementation in &self.implementations {
            implementation.initialize(context.clone()).await?;
        }
        Ok(())
    }

    pub(super) async fn shutdown(&self) -> Result<()> {
        let mut errors = Vec::new();
        for implementation in self.implementations.iter().rev() {
            // 一个原生实现 panic 也不能阻止其他执行器释放资源。
            let result = AssertUnwindSafe(async { implementation.shutdown().await })
                .catch_unwind()
                .await
                .unwrap_or_else(|_| Err(anyhow::anyhow!("整包执行器关闭时 panic")));
            if let Err(error) = result {
                errors.push(format!("{error:#}"));
            }
        }
        ensure!(
            errors.is_empty(),
            "关闭整包执行器失败: {}",
            errors.join("; ")
        );
        Ok(())
    }
}

/// 运行中实例的路由绑定；身份来自租户和来源，执行器 TypeId 不进入持久化数据。
pub(super) struct ActiveExecution {
    pub digest: String,
    pub executor: Arc<dyn ComponentExecutor>,
}
