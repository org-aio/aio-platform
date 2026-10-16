//! Bevy 风格的应用插件与 Dill 聚合入口。

use std::{
    any::{Any, TypeId},
    collections::{HashMap, HashSet},
    future::Future,
    marker::PhantomData,
    pin::Pin,
    sync::Arc,
};

use anyhow::{Context as _, Result, ensure};
use dill::{AllOf, Catalog};

/// 插件构建过程。
pub type PluginFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

/// 指定应用状态的动态插件。
pub type DynPlugin<T> = Arc<dyn Plugin<T>>;

/// 通过修改应用状态安装一组能力。
pub trait Plugin<T>: Any + Send + Sync {
    /// 构建插件；异步返回用于覆盖数据库和文件系统初始化。
    fn build<'a>(&'a self, app: &'a mut T) -> PluginFuture<'a>;

    /// 构建前必须存在的具体插件类型；内核在执行任何插件前检查完整依赖图。
    fn dependencies(&self) -> Vec<TypeId> {
        Vec::new()
    }

    /// 可选的人类可读说明，只用于日志和诊断。
    fn comment(&self) -> &'static str {
        ""
    }
}

/// 按依赖顺序构建应用状态；无依赖关系的插件保留装配顺序。
pub struct App<T> {
    state: T,
    unique_plugin_types: HashSet<TypeId>,
    plugin_order: Vec<TypeId>,
}

impl<T: 'static> App<T> {
    #[must_use]
    pub fn new(state: T) -> Self {
        Self {
            state,
            unique_plugin_types: HashSet::new(),
            plugin_order: Vec::new(),
        }
    }

    pub async fn add_plugins(
        &mut self,
        plugins: impl IntoIterator<Item = DynPlugin<T>>,
    ) -> Result<()> {
        let plugins = plugins.into_iter().collect::<Vec<_>>();
        let plugins = self.plan_plugins(plugins, &HashMap::new())?;

        for plugin in plugins {
            let type_id = plugin.as_ref().type_id();
            plugin.build(&mut self.state).await.with_context(|| {
                let comment = plugin.comment();
                if comment.is_empty() {
                    format!("构建插件失败: {type_id:?}")
                } else {
                    format!("构建插件失败: {comment} ({type_id:?})")
                }
            })?;
            self.unique_plugin_types.insert(type_id);
            self.plugin_order.push(type_id);
        }
        Ok(())
    }

    /// 验证完整依赖图并返回顺序；宿主可补充由接口声明导出的 TypeId 依赖。
    pub fn plan_plugins(
        &self,
        plugins: Vec<DynPlugin<T>>,
        additional: &HashMap<TypeId, Vec<TypeId>>,
    ) -> Result<Vec<DynPlugin<T>>> {
        let mut pending_unique_types = HashSet::new();
        for plugin in &plugins {
            let type_id = plugin.as_ref().type_id();
            ensure!(
                !self.unique_plugin_types.contains(&type_id),
                "插件类型已添加到应用: {type_id:?}"
            );
            ensure!(
                pending_unique_types.insert(type_id),
                "插件组包含重复插件类型: {type_id:?}"
            );
        }
        let available = self
            .unique_plugin_types
            .union(&pending_unique_types)
            .copied()
            .collect::<HashSet<_>>();
        let mut pending = Vec::new();
        for plugin in plugins {
            let mut dependencies = plugin.dependencies();
            if let Some(extra) = additional.get(&plugin.as_ref().type_id()) {
                dependencies.extend(extra);
            }
            for dependency in &dependencies {
                ensure!(
                    available.contains(dependency),
                    "插件依赖缺失: {:?} -> {dependency:?}",
                    plugin.as_ref().type_id()
                );
            }
            pending.push((plugin, dependencies));
        }
        let mut built = self.unique_plugin_types.clone();
        let mut ordered = Vec::new();
        while !pending.is_empty() {
            let position = pending
                .iter()
                .position(|(_, dependencies)| {
                    dependencies
                        .iter()
                        .all(|dependency| built.contains(dependency))
                })
                .context("插件依赖形成循环，不能启动")?;
            let (plugin, _) = pending.remove(position);
            built.insert(plugin.as_ref().type_id());
            ordered.push(plugin);
        }
        Ok(ordered)
    }

    /// 返回已经构建成功的具体插件类型顺序，供宿主按依赖反序释放资源。
    pub fn plugin_order(&self) -> &[TypeId] {
        &self.plugin_order
    }

    #[must_use]
    pub fn into_inner(self) -> T {
        self.state
    }
}

/// 声明一组有确定顺序的应用插件。
pub trait PluginGroup<T: 'static> {
    fn build(self) -> PluginGroupBuilder<T>;
}

/// 使用具体 Rust 类型构建插件组。
pub struct PluginGroupBuilder<T> {
    plugin_types: Vec<TypeId>,
    marker: PhantomData<fn(&mut T)>,
}

impl<T: 'static> Default for PluginGroupBuilder<T> {
    fn default() -> Self {
        Self {
            plugin_types: Vec::new(),
            marker: PhantomData,
        }
    }
}

impl<T: 'static> PluginGroupBuilder<T> {
    #[must_use]
    pub fn add<P>(mut self) -> Self
    where
        P: Plugin<T>,
    {
        self.plugin_types.push(TypeId::of::<P>());
        self
    }

    pub fn resolve(self, catalog: &Catalog) -> Result<Vec<DynPlugin<T>>> {
        let mut expected = HashSet::new();
        for type_id in &self.plugin_types {
            ensure!(expected.insert(*type_id), "插件组包含重复类型: {type_id:?}");
        }

        let registrations = catalog
            .get::<AllOf<dyn Plugin<T>>>()
            .context("从 Dill 聚合应用插件失败")?;
        let mut resolved = HashMap::new();
        for plugin in registrations {
            let type_id = plugin.as_ref().type_id();
            ensure!(
                resolved.insert(type_id, plugin).is_none(),
                "Dill 中重复注册插件类型: {type_id:?}"
            );
        }
        let registered_types = resolved.keys().copied().collect::<HashSet<_>>();
        ensure!(
            registered_types == expected,
            "Dill 插件集合与 PluginGroup 不一致: 注册={registered_types:?}, 分组={expected:?}"
        );

        self.plugin_types
            .into_iter()
            .map(|type_id| {
                resolved
                    .remove(&type_id)
                    .with_context(|| format!("插件组解析结果缺少类型: {type_id:?}"))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use dill::Catalog;

    use super::*;

    #[derive(Default)]
    struct TestState {
        builds: Vec<&'static str>,
    }

    struct First;
    struct Second;

    impl Plugin<TestState> for First {
        fn build<'a>(&'a self, app: &'a mut TestState) -> PluginFuture<'a> {
            Box::pin(async move {
                app.builds.push("first");
                Ok(())
            })
        }
    }

    impl Plugin<TestState> for Second {
        fn build<'a>(&'a self, app: &'a mut TestState) -> PluginFuture<'a> {
            Box::pin(async move {
                app.builds.push("second");
                Ok(())
            })
        }
    }

    struct TestPlugins;

    impl PluginGroup<TestState> for TestPlugins {
        fn build(self) -> PluginGroupBuilder<TestState> {
            PluginGroupBuilder::default().add::<Second>().add::<First>()
        }
    }

    fn catalog() -> Catalog {
        Catalog::builder()
            .add_value(First)
            .bind::<dyn Plugin<TestState>, First>()
            .add_value(Second)
            .bind::<dyn Plugin<TestState>, Second>()
            .build()
    }

    #[tokio::test]
    async fn resolves_plugins_in_group_order_by_type_id() -> Result<()> {
        let catalog = catalog();
        let plugins = TestPlugins.build().resolve(&catalog)?;
        let mut app = App::new(TestState::default());

        app.add_plugins(plugins).await?;

        assert_eq!(app.into_inner().builds, ["second", "first"]);
        Ok(())
    }

    #[tokio::test]
    async fn rejects_duplicate_unique_plugin_before_build() -> Result<()> {
        let plugin = Arc::new(First) as DynPlugin<TestState>;
        let mut app = App::new(TestState::default());

        let error = app
            .add_plugins([Arc::clone(&plugin), plugin])
            .await
            .err()
            .context("重复插件必须被拒绝")?;

        assert!(error.to_string().contains("重复插件"));
        assert!(app.into_inner().builds.is_empty());
        Ok(())
    }

    struct Dependent;
    impl Plugin<TestState> for Dependent {
        fn dependencies(&self) -> Vec<TypeId> {
            vec![TypeId::of::<First>()]
        }
        fn build<'a>(&'a self, app: &'a mut TestState) -> PluginFuture<'a> {
            Box::pin(async move {
                app.builds.push("dependent");
                Ok(())
            })
        }
    }

    struct Cyclic;
    impl Plugin<TestState> for Cyclic {
        fn dependencies(&self) -> Vec<TypeId> {
            vec![TypeId::of::<Cyclic>()]
        }
        fn build<'a>(&'a self, app: &'a mut TestState) -> PluginFuture<'a> {
            Box::pin(async move {
                app.builds.push("cyclic");
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn dependency_graph_orders_plugins_and_accepts_existing_dependencies() -> Result<()> {
        let mut app = App::new(TestState::default());
        let plugins: Vec<DynPlugin<TestState>> =
            vec![Arc::new(Dependent), Arc::new(Second), Arc::new(First)];
        app.add_plugins(plugins).await?;
        assert_eq!(app.into_inner().builds, ["second", "first", "dependent"]);
        let mut app = App::new(TestState::default());
        app.add_plugins(vec![Arc::new(First) as DynPlugin<TestState>])
            .await?;
        app.add_plugins(vec![Arc::new(Dependent) as DynPlugin<TestState>])
            .await?;
        assert_eq!(app.into_inner().builds, ["first", "dependent"]);
        Ok(())
    }

    #[test]
    fn additional_dependencies_order_plugins_and_reject_cycles_before_build() -> Result<()> {
        let app = App::new(TestState::default());
        let plugins = vec![Arc::new(First) as DynPlugin<TestState>, Arc::new(Second)];
        let mut dependencies =
            HashMap::from([(TypeId::of::<First>(), vec![TypeId::of::<Second>()])]);
        let planned = app.plan_plugins(plugins.clone(), &dependencies)?;
        assert_eq!(
            planned
                .iter()
                .map(|plugin| plugin.as_ref().type_id())
                .collect::<Vec<_>>(),
            [TypeId::of::<Second>(), TypeId::of::<First>()]
        );
        dependencies.insert(TypeId::of::<Second>(), vec![TypeId::of::<First>()]);
        assert!(app.plan_plugins(plugins.clone(), &dependencies).is_err());
        dependencies.insert(TypeId::of::<Second>(), vec![TypeId::of::<u64>()]);
        assert!(app.plan_plugins(plugins, &dependencies).is_err());
        assert!(app.into_inner().builds.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn rejects_missing_dependencies_and_cycles_before_any_build() {
        for invalid in [
            Arc::new(Dependent) as DynPlugin<TestState>,
            Arc::new(Cyclic),
        ] {
            let mut app = App::new(TestState::default());
            assert!(
                app.add_plugins(vec![Arc::new(Second), invalid])
                    .await
                    .is_err()
            );
            assert!(app.into_inner().builds.is_empty());
        }
    }
}
