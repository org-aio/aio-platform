use super::model::WebviewRuntime;
use anyhow::{Context, Result, ensure};
use std::{
    any::Any,
    collections::HashSet,
    sync::{Arc, OnceLock},
};

/// 设备插件向同一 Dill 容器贡献通道契约；提供者身份只使用具体类型的 TypeId。
pub trait WebviewProtocolProvider: Any + Send + Sync {
    fn protocols(&self) -> Vec<WebviewRuntime>;
}

/// 每代装配固定一份经过校验的协议快照；移除提供者会重建依赖它的路由插件。
pub(crate) struct WebviewProtocols {
    providers: Vec<Arc<dyn WebviewProtocolProvider>>,
    snapshot: OnceLock<Result<Vec<WebviewRuntime>, String>>,
}

#[dill::component(pub(crate))]
#[dill::scope(dill::Singleton)]
impl WebviewProtocols {
    fn new(providers: Vec<Arc<dyn WebviewProtocolProvider>>) -> Self {
        Self {
            providers,
            snapshot: OnceLock::new(),
        }
    }
}

impl WebviewProtocols {
    pub fn validate(&self) -> Result<()> {
        self.entries()?;
        Ok(())
    }

    fn entries(&self) -> Result<&Vec<WebviewRuntime>> {
        self.snapshot
            .get_or_init(|| self.collect().map_err(|error| error.to_string()))
            .as_ref()
            .map_err(|error| anyhow::anyhow!(error.clone()))
    }

    fn collect(&self) -> Result<Vec<WebviewRuntime>> {
        let mut types = HashSet::new();
        let mut protocols = HashSet::new();
        let mut capabilities = HashSet::new();
        let mut entries = Vec::new();
        for provider in &self.providers {
            ensure!(
                types.insert(provider.as_ref().type_id()),
                "设备通道提供者类型重复"
            );
            let declared = provider.protocols();
            ensure!(!declared.is_empty(), "设备通道提供者没有声明协议");
            for runtime in declared {
                validate_runtime(&runtime)?;
                ensure!(
                    protocols.insert(runtime.protocol.clone()),
                    "设备通道协议声明冲突"
                );
                for capability in
                    std::iter::once(&runtime.capability).chain(runtime.headless_capability.iter())
                {
                    ensure!(
                        capabilities.insert(capability.clone()),
                        "设备通道能力声明冲突"
                    );
                }
                entries.push(runtime);
            }
        }
        Ok(entries)
    }

    pub fn runtime(&self, protocol: &str) -> Result<WebviewRuntime> {
        self.entries()?
            .iter()
            .find(|runtime| runtime.protocol == protocol)
            .cloned()
            .context("设备通道协议未装配")
    }

    pub fn capabilities(&self) -> Result<Vec<String>> {
        Ok(self
            .entries()?
            .iter()
            .map(|runtime| runtime.capability.clone())
            .collect())
    }

    pub fn enabled(&self, capabilities: &[String]) -> Result<bool> {
        Ok(self
            .entries()?
            .iter()
            .any(|runtime| capabilities.contains(&runtime.capability)))
    }

    pub fn browser_runtime(&self, capabilities: &[String]) -> Result<Option<WebviewRuntime>> {
        let mut matching = self.entries()?.iter().filter(|runtime| {
            runtime.document.is_some() && capabilities.contains(&runtime.capability)
        });
        let selected = matching.next().cloned();
        ensure!(matching.next().is_none(), "插件必须声明唯一的网页运行时");
        Ok(selected)
    }
}

/// 能力只允许协议标识的字符，防止错误声明污染其他权限；不按名称识别提供者。
pub(super) fn validate_runtime(runtime: &WebviewRuntime) -> Result<()> {
    for value in std::iter::once(&runtime.protocol)
        .chain(std::iter::once(&runtime.capability))
        .chain(runtime.headless_capability.iter())
    {
        ensure!(
            !value.is_empty()
                && value.len() <= 64
                && value.as_bytes()[0].is_ascii_lowercase()
                && value.bytes().all(|c| c.is_ascii_lowercase()
                    || c.is_ascii_digit()
                    || matches!(c, b'.' | b'-' | b'_')),
            "设备通道协议或能力格式无效"
        );
    }
    ensure!(
        runtime.headless_capability.as_ref() != Some(&runtime.capability),
        "设备通道的附加能力必须独立"
    );
    Ok(())
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
