use std::{collections::BTreeMap, sync::Arc};

use crate::{
    ChildrenConstraint, ComponentCatalog, ComponentContract, ComponentPropertyContract,
    EventContract, ValueType,
};
use anyhow::{Context, Result, bail, ensure};
use rudi::Context as RudiContext;

use crate::{
    ComponentCatalogEntry, ComponentDefinition, ComponentEventSpec, ComponentPropertySpec,
    ComponentRenderContext, ComponentShape, ComponentSpec, component::DynDynamicComponentProvider,
};

/// 从 Rudi provider 构建出的不可变组件定义。
#[derive(Clone, Debug)]
pub struct IndexedComponent {
    canonical_id: String,
    dsl_name: String,
    definition: ComponentDefinition,
    properties: BTreeMap<String, ComponentPropertySpec>,
    events: BTreeMap<String, ComponentEventSpec>,
    semantic_names: Vec<String>,
    provider: DynDynamicComponentProvider,
}

impl IndexedComponent {
    #[must_use]
    pub fn canonical_id(&self) -> &str {
        &self.canonical_id
    }

    #[must_use]
    pub fn dsl_name(&self) -> &str {
        &self.dsl_name
    }

    #[must_use]
    pub const fn shape(&self) -> ComponentShape {
        self.definition.shape
    }

    #[must_use]
    pub fn spec(&self) -> &ComponentSpec {
        &self.definition.spec
    }

    #[must_use]
    pub fn definition(&self) -> &ComponentDefinition {
        &self.definition
    }

    #[must_use]
    pub fn properties(&self) -> &BTreeMap<String, ComponentPropertySpec> {
        &self.properties
    }

    #[must_use]
    pub fn events(&self) -> &BTreeMap<String, ComponentEventSpec> {
        &self.events
    }

    #[must_use]
    pub fn semantic_names(&self) -> &[String] {
        &self.semantic_names
    }

    pub fn render(&self, context: ComponentRenderContext) -> dioxus::prelude::Element {
        self.provider.render(context)
    }
}

/// 由当前 Rudi Context 中全部组件 provider 派生出的只读查询索引。
#[derive(Clone, Debug, Default)]
pub struct ComponentIndex {
    by_dsl_name: BTreeMap<String, Arc<IndexedComponent>>,
    by_canonical_id: BTreeMap<String, Arc<IndexedComponent>>,
}

impl ComponentIndex {
    /// 从现有 Rudi Context 收集全部远程组件。
    pub fn from_context(context: &mut RudiContext) -> Result<Self> {
        let provider_names = context
            .get_providers_by_type::<DynDynamicComponentProvider>()
            .into_iter()
            .map(|provider| provider.definition().key.name.to_string())
            .collect::<Vec<_>>();
        ensure!(!provider_names.is_empty(), "Rudi 中没有注册远程 UI 组件");

        let mut index = Self::default();
        for provider_name in provider_names {
            let component = context
                .resolve_option_with_name::<DynDynamicComponentProvider>(provider_name.clone())
                .with_context(|| format!("无法解析远程 UI 组件 provider: {provider_name}"))?;
            let definition = component.definition();
            let properties = component.properties();
            let events = component.events();
            let semantic_names = component.semantic_names();
            let dsl_name = dsl_name_from_provider_name(&provider_name)?;
            index.insert(
                format!("ui.{dsl_name}"),
                dsl_name,
                definition,
                properties,
                events,
                semantic_names,
                component,
            )?;
        }
        Ok(index)
    }

    /// 按 DSL 中的短名称解析组件。
    pub fn resolve(&self, dsl_name: &str) -> Result<&IndexedComponent> {
        self.by_dsl_name
            .get(dsl_name)
            .map(Arc::as_ref)
            .with_context(|| format!("未注册的远程 UI 组件: {dsl_name}"))
    }

    /// 返回浏览器 renderer 使用的 canonical ID 到样式定义映射。
    #[must_use]
    pub fn browser_catalog(&self) -> BTreeMap<String, ComponentCatalogEntry> {
        self.by_canonical_id
            .iter()
            .map(|(id, component)| {
                (
                    id.clone(),
                    ComponentCatalogEntry {
                        shape: component.shape(),
                        spec: component.spec().clone(),
                        properties: component.properties.clone(),
                        events: component.events.clone(),
                    },
                )
            })
            .collect()
    }

    /// 返回自然编译器可消费的组件语义目录。
    pub fn semantic_catalog(&self) -> Vec<(String, String, Vec<String>)> {
        self.by_canonical_id
            .values()
            .map(|component| {
                (
                    component.canonical_id.clone(),
                    component.dsl_name.clone(),
                    component.semantic_names.clone(),
                )
            })
            .collect()
    }

    /// 把 Rudi Provider 元数据投影为 ProgramGraph 编译目录。
    #[must_use]
    pub fn program_catalog(&self) -> ComponentCatalog {
        ComponentCatalog {
            components: self
                .by_canonical_id
                .iter()
                .map(|(canonical_id, component)| {
                    let properties = component
                        .properties()
                        .iter()
                        .map(|(name, property)| {
                            (
                                name.clone(),
                                ComponentPropertyContract {
                                    value_type: property_value_type(property.kind),
                                    required: property.required,
                                    choices: property.choices.clone(),
                                },
                            )
                        })
                        .collect();
                    let events = component
                        .events()
                        .iter()
                        .map(|(name, event)| {
                            let payload = event
                                .payload
                                .iter()
                                .map(|(name, kind)| (name.clone(), property_value_type(*kind)))
                                .collect();
                            (name.clone(), EventContract { payload })
                        })
                        .collect();
                    let children = match component.shape() {
                        ComponentShape::Leaf => ChildrenConstraint::None,
                        ComponentShape::Container | ComponentShape::Dual => ChildrenConstraint::Any,
                    };
                    (
                        canonical_id.clone(),
                        ComponentContract {
                            canonical_id: canonical_id.clone(),
                            properties,
                            events,
                            children,
                        },
                    )
                })
                .collect(),
        }
    }

    /// 按正式配置中的 canonical ID 解析组件。
    pub fn resolve_canonical(&self, canonical_id: &str) -> Result<&IndexedComponent> {
        self.by_canonical_id
            .get(canonical_id)
            .map(Arc::as_ref)
            .with_context(|| format!("未注册的远程 UI 组件路径: {canonical_id}"))
    }

    fn insert(
        &mut self,
        canonical_id: String,
        dsl_name: String,
        definition: ComponentDefinition,
        properties: BTreeMap<String, ComponentPropertySpec>,
        events: BTreeMap<String, ComponentEventSpec>,
        semantic_names: Vec<String>,
        provider: DynDynamicComponentProvider,
    ) -> Result<()> {
        if let Some(existing) = self.by_dsl_name.get(&dsl_name) {
            bail!(
                "远程 UI 组件短名称冲突: {dsl_name}; {} 与 {canonical_id}",
                existing.canonical_id()
            );
        }

        let component = Arc::new(IndexedComponent {
            canonical_id: canonical_id.clone(),
            dsl_name: dsl_name.clone(),
            definition,
            properties,
            events,
            semantic_names,
            provider,
        });
        self.by_dsl_name.insert(dsl_name, Arc::clone(&component));
        self.by_canonical_id.insert(canonical_id, component);
        Ok(())
    }
}

fn property_value_type(kind: crate::ComponentPropertyKind) -> ValueType {
    match kind {
        crate::ComponentPropertyKind::Boolean => ValueType::Boolean,
        crate::ComponentPropertyKind::Number => ValueType::Decimal,
        crate::ComponentPropertyKind::Text
        | crate::ComponentPropertyKind::Choice
        | crate::ComponentPropertyKind::Action => ValueType::Text,
        crate::ComponentPropertyKind::Json => ValueType::Any,
    }
}

fn dsl_name_from_provider_name(provider_name: &str) -> Result<String> {
    let module_name = provider_name
        .rsplit("::")
        .next()
        .filter(|value| !value.is_empty())
        .with_context(|| format!("组件 provider 路径无效: {provider_name}"))?;
    let mut normalized = String::with_capacity(module_name.len());
    for (index, character) in module_name.chars().enumerate() {
        if character == '_' {
            normalized.push('-');
            continue;
        }
        if character.is_ascii_uppercase()
            && index > 0
            && normalized.chars().last().is_some_and(|value| value != '-')
        {
            normalized.push('-');
        }
        normalized.push(character.to_ascii_lowercase());
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rudi_provider_path_derives_stable_component_id() -> Result<()> {
        let mut context = RudiContext::auto_register();
        let components = ComponentIndex::from_context(&mut context)?;
        let card = components.resolve("card")?;

        // Rust 模块移动不能改变数据库中持久化的组件身份。
        assert_eq!(card.canonical_id(), "ui.card");
        assert!(
            components
                .browser_catalog()
                .contains_key(card.canonical_id())
        );
        Ok(())
    }
}
