//! 记录插件的 Dill 注册动作；重建组合时保留未受影响的原始 Builder 与单例缓存。

use std::{
    any::{Any, TypeId},
    collections::HashSet,
    marker::Unsize,
    sync::Arc,
};

use anyhow::{Result, ensure};
use dill::{
    Builder, BuilderExt, Catalog, CatalogBuilder, Component, DependencyInfo, InjectionContext,
    InjectionError, TypeInfo, TypedBuilder,
};

type Registration = Arc<dyn Fn(&mut CatalogBuilder) + Send + Sync>;

/// 服务仍由 Dill 创建、注入和缓存；此处只保存可组合的装配动作及其归属信息。
#[derive(Clone, Default)]
pub struct ServiceBindings {
    registrations: Vec<Registration>,
    pub(super) implementations: HashSet<TypeId>,
    pub(super) interfaces: HashSet<TypeId>,
    pub(super) dependencies: HashSet<TypeId>,
    errors: Vec<String>,
}

impl ServiceBindings {
    /// 创建当前插件的空装配记录。
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册 Dill 组件及其默认接口，保留原始 Builder 的单例缓存。
    pub fn add<C: Component + 'static>(&mut self) -> &mut Self {
        self.add_builder(C::builder())
    }

    /// 注册当前配方拥有的值；有关闭责任的异步资源应使用 declare_resource。
    pub fn add_value<T: Send + Sync + 'static>(&mut self, value: T) -> &mut Self {
        self.add_builder(Arc::new(value))
    }

    /// 接收 Dill Builder；传入 Arc<T> 时绑定同一个 T 实例。
    pub fn add_builder<T, B>(&mut self, builder: B) -> &mut Self
    where
        T: Send + Sync + 'static,
        B: TypedBuilder<T> + 'static,
    {
        let identity = TypeId::of::<T>();
        if !self.implementations.insert(identity) {
            self.errors
                .push(format!("插件重复声明服务实现: {identity:?}"));
            return self;
        }
        self.interfaces.insert(identity);
        self.interfaces.extend(
            builder
                .interfaces_get_all()
                .into_iter()
                .map(|interface| interface.id),
        );
        self.dependencies.extend(
            builder
                .dependencies_get_all()
                .into_iter()
                .filter(|dependency| !dependency.is_bound)
                .map(|dependency| dependency.iface.id),
        );
        let builder = Arc::new(builder);
        self.registrations.push(Arc::new(move |catalog| {
            catalog.add_builder(SharedBuilder(builder.clone()));
        }));
        self
    }

    /// 为当前插件提供的具体类型增加接口绑定。
    pub fn bind<I, T>(&mut self) -> &mut Self
    where
        I: ?Sized + 'static,
        T: Send + Sync + Unsize<I> + 'static,
    {
        if !self.implementations.contains(&TypeId::of::<T>()) {
            self.errors.push(format!(
                "插件不能为未由自己提供的实现绑定接口: {:?}",
                TypeId::of::<T>()
            ));
            return self;
        }
        self.interfaces.insert(TypeId::of::<I>());
        self.registrations.push(Arc::new(|catalog| {
            catalog.bind::<I, T>();
        }));
        self
    }

    pub(super) fn contains<T: 'static>(&self) -> bool {
        self.implementations.contains(&TypeId::of::<T>())
    }

    pub(super) fn register(&self, catalog: &mut CatalogBuilder) -> Result<()> {
        ensure!(self.errors.is_empty(), "{}", self.errors.join("; "));
        for registration in &self.registrations {
            registration(catalog);
        }
        Ok(())
    }
}

// Arc<B> 在 Dill 中表示 B 自身的实例；这里显式复用原 Builder 的缓存与绑定行为。
struct SharedBuilder<B>(Arc<B>);

impl<B: Builder> Builder for SharedBuilder<B> {
    fn instance_type(&self) -> TypeInfo {
        self.0.as_ref().instance_type()
    }
    fn scope_type(&self) -> TypeInfo {
        self.0.as_ref().scope_type()
    }
    fn interfaces(&self, callback: &mut dyn FnMut(&TypeInfo) -> bool) {
        self.0.as_ref().interfaces(callback);
    }
    fn dependencies(&self, callback: &mut dyn FnMut(&DependencyInfo) -> bool) {
        self.0.as_ref().dependencies(callback);
    }
    fn metadata<'a>(&'a self, callback: &mut dyn FnMut(&'a dyn Any) -> bool) {
        self.0.as_ref().metadata(callback);
    }
    fn get_any(
        &self,
        catalog: &Catalog,
        context: &InjectionContext,
    ) -> Result<Arc<dyn Any + Send + Sync>, InjectionError> {
        self.0.as_ref().get_any(catalog, context)
    }
}

impl<T, B> TypedBuilder<T> for SharedBuilder<B>
where
    T: Send + Sync + 'static,
    B: TypedBuilder<T>,
{
    fn get_with_context(
        &self,
        catalog: &Catalog,
        context: &InjectionContext,
    ) -> Result<Arc<T>, InjectionError> {
        self.0.as_ref().get_with_context(catalog, context)
    }
    fn bind_interfaces(&self, catalog: &mut CatalogBuilder) {
        self.0.as_ref().bind_interfaces(catalog);
    }
}
