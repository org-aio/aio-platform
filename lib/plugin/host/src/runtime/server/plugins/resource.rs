//! 将异步初始化的资源接入同一个 Dill 容器，业务服务仍直接注入资源类型。

use std::{
    any::Any,
    sync::{Arc, OnceLock},
};

use anyhow::{Result, ensure};
use dill::{
    Builder, Catalog, CatalogBuilder, DependencyInfo, InjectionContext, InjectionError, Singleton,
    TypeInfo, TypedBuilder,
};

/// 插件初始化时提供资源，初始化失败时停止钩子可检查是否需要释放。
pub struct Resource<T> {
    value: OnceLock<Arc<T>>,
}

impl<T> Resource<T> {
    pub fn provide(&self, value: Arc<T>) -> Result<()> {
        ensure!(self.value.set(value).is_ok(), "资源不能重复初始化");
        Ok(())
    }

    pub fn initialized(&self) -> Option<Arc<T>> {
        self.value.get().cloned()
    }
}

pub(super) fn declare<T: Send + Sync + 'static>(
    services: &mut super::ServiceBindings,
) -> Result<()> {
    // 在注册前检查具体类型，让重复资源声明返回错误而不是触发 Dill 的 panic。
    ensure!(!services.contains::<T>(), "资源类型已注册");
    ensure!(!services.contains::<Resource<T>>(), "资源槽已注册");
    let resource = Arc::new(Resource::<T> {
        value: OnceLock::new(),
    });
    services.add_builder(resource.clone());
    services.add_builder(ResourceBuilder(resource));
    Ok(())
}

struct ResourceBuilder<T>(Arc<Resource<T>>);

impl<T: Send + Sync + 'static> Builder for ResourceBuilder<T> {
    fn instance_type(&self) -> TypeInfo {
        TypeInfo::of::<T>()
    }
    fn scope_type(&self) -> TypeInfo {
        TypeInfo::of::<Singleton>()
    }
    fn interfaces(&self, _: &mut dyn FnMut(&TypeInfo) -> bool) {}
    fn dependencies(&self, _: &mut dyn FnMut(&DependencyInfo) -> bool) {}
    fn metadata<'a>(&'a self, _: &mut dyn FnMut(&'a dyn Any) -> bool) {}
    fn get_any(
        &self,
        catalog: &Catalog,
        context: &InjectionContext,
    ) -> Result<Arc<dyn Any + Send + Sync>, InjectionError> {
        Ok(self.get_with_context(catalog, context)?)
    }
}

impl<T: Send + Sync + 'static> TypedBuilder<T> for ResourceBuilder<T> {
    fn get_with_context(
        &self,
        _: &Catalog,
        context: &InjectionContext,
    ) -> Result<Arc<T>, InjectionError> {
        self.0
            .initialized()
            .ok_or_else(|| InjectionError::unregistered::<T>(context))
    }
    fn bind_interfaces(&self, _: &mut CatalogBuilder) {}
}
