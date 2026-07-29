use std::{collections::BTreeMap, sync::Arc};

use crate::{Alignment, ComponentStyle, SizeConstraint, SpacingToken, SymbolId};
pub(crate) use crate::{ComponentBehavior, ComponentPropertyKind, ComponentShape};
use dioxus::prelude::{Callback, Element};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 单个组件属性的编辑和校验约束。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComponentPropertySpec {
    pub kind: ComponentPropertyKind,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
}

/// 组件事件向动作系统暴露的载荷字段。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComponentEventSpec {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub payload: BTreeMap<String, ComponentPropertyKind>,
}

/// Studio 目录使用的编辑器标签与 Provider 受控样式元数据。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComponentSpec {
    pub html_tag: String,
    pub class_name: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub variants: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_variant: Option<String>,
    #[serde(default, skip_serializing_if = "ComponentBehavior::is_generic")]
    pub behavior: ComponentBehavior,
}

/// 组件实例提供给查询索引的完整定义。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComponentDefinition {
    pub shape: ComponentShape,
    pub spec: ComponentSpec,
}

/// 发送给 Studio 组件目录的完整元数据。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComponentCatalogEntry {
    pub shape: ComponentShape,
    pub spec: ComponentSpec,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, ComponentPropertySpec>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub events: BTreeMap<String, ComponentEventSpec>,
}

/// Dioxus 组件事件统一进入客户端 Graph VM。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DynamicComponentEvent {
    pub component_id: SymbolId,
    pub event: String,
    #[serde(default)]
    pub payload: BTreeMap<String, Value>,
}

/// Provider 渲染函数收到的已解析属性与递归子树。
pub struct ComponentRenderContext {
    pub component_id: SymbolId,
    pub properties: BTreeMap<String, Value>,
    pub content: Option<String>,
    pub children: Element,
    pub dispatch: Callback<DynamicComponentEvent>,
    pub style: ComponentStyle,
}

impl ComponentRenderContext {
    #[must_use]
    pub fn text(&self, name: &str) -> String {
        self.properties
            .get(name)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    }

    #[must_use]
    pub fn boolean(&self, name: &str) -> bool {
        self.properties
            .get(name)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    #[must_use]
    pub fn number(&self, name: &str) -> f64 {
        self.properties
            .get(name)
            .and_then(Value::as_f64)
            .unwrap_or_default()
    }

    #[must_use]
    pub fn body_text(&self) -> String {
        self.content.clone().unwrap_or_default()
    }

    #[must_use]
    pub fn layout_class(&self) -> String {
        let mut classes = Vec::new();
        if let Some(spacing) = self.style.spacing {
            classes.push(
                match spacing {
                    SpacingToken::None => "gap-0",
                    SpacingToken::Xs => "gap-1",
                    SpacingToken::Sm => "gap-2",
                    SpacingToken::Md => "gap-4",
                    SpacingToken::Lg => "gap-6",
                    SpacingToken::Xl => "gap-8",
                }
                .to_owned(),
            );
        }
        if let Some(width) = &self.style.width {
            classes.push(size_class(width, true));
        }
        if let Some(height) = &self.style.height {
            classes.push(size_class(height, false));
        }
        if let Some(alignment) = self.style.alignment {
            classes.push(
                match alignment {
                    Alignment::Start => "items-start",
                    Alignment::Center => "items-center",
                    Alignment::End => "items-end",
                    Alignment::Stretch => "items-stretch",
                    Alignment::SpaceBetween => "justify-between",
                }
                .to_owned(),
            );
        }
        classes.join(" ")
    }
}

fn size_class(size: &SizeConstraint, width: bool) -> String {
    match (size, width) {
        (SizeConstraint::Auto, true) => "w-auto".to_owned(),
        (SizeConstraint::Auto, false) => "h-auto".to_owned(),
        (SizeConstraint::Full, true) => "w-full".to_owned(),
        (SizeConstraint::Full, false) => "h-full".to_owned(),
        (SizeConstraint::Content, true) => "w-fit".to_owned(),
        (SizeConstraint::Content, false) => "h-fit".to_owned(),
        (SizeConstraint::Fraction(value), true) => format!("col-span-{value}"),
        (SizeConstraint::Fraction(value), false) => format!("row-span-{value}"),
        (SizeConstraint::Token(value), _) => value.clone(),
    }
}

/// Rudi 管理的唯一动态 Dioxus 组件 Provider。
pub trait DynamicComponentProvider: Send + Sync + std::fmt::Debug {
    fn definition(&self) -> ComponentDefinition;

    fn render(&self, context: ComponentRenderContext) -> Element;

    /// 提供给自然编译器的母语名称和别名。
    fn semantic_names(&self) -> Vec<String> {
        Vec::new()
    }

    fn properties(&self) -> BTreeMap<String, ComponentPropertySpec> {
        BTreeMap::new()
    }

    fn events(&self) -> BTreeMap<String, ComponentEventSpec> {
        BTreeMap::new()
    }
}

pub type DynDynamicComponentProvider = Arc<dyn DynamicComponentProvider>;

pub(crate) fn bind_dynamic_component<T>(component: T) -> DynDynamicComponentProvider
where
    T: DynamicComponentProvider + 'static,
{
    Arc::new(component)
}

pub(crate) fn spec(html_tag: &str, class_name: &str) -> ComponentSpec {
    ComponentSpec {
        html_tag: html_tag.to_string(),
        class_name: class_name.to_string(),
        variants: BTreeMap::new(),
        default_variant: None,
        behavior: ComponentBehavior::Generic,
    }
}

pub(crate) fn spec_with_behavior(
    html_tag: &str,
    class_name: &str,
    behavior: ComponentBehavior,
) -> ComponentSpec {
    ComponentSpec {
        html_tag: html_tag.to_string(),
        class_name: class_name.to_string(),
        variants: BTreeMap::new(),
        default_variant: None,
        behavior,
    }
}

pub(crate) fn spec_with_variants(
    html_tag: &str,
    class_name: &str,
    variants: &[(&str, &str)],
    default_variant: &str,
    behavior: ComponentBehavior,
) -> ComponentSpec {
    let variants = variants
        .iter()
        .map(|(name, class_name)| (name.to_string(), class_name.to_string()))
        .collect();
    ComponentSpec {
        html_tag: html_tag.to_string(),
        class_name: class_name.to_string(),
        variants,
        default_variant: Some(default_variant.to_string()),
        behavior,
    }
}

pub(crate) const fn property(kind: ComponentPropertyKind, required: bool) -> ComponentPropertySpec {
    ComponentPropertySpec {
        kind,
        required,
        choices: Vec::new(),
    }
}

pub(crate) fn properties(
    values: &[(&str, ComponentPropertySpec)],
) -> BTreeMap<String, ComponentPropertySpec> {
    values
        .iter()
        .map(|(name, property)| ((*name).to_string(), property.clone()))
        .collect()
}

pub(crate) fn choice_property(choices: &[&str]) -> ComponentPropertySpec {
    ComponentPropertySpec {
        kind: ComponentPropertyKind::Choice,
        required: false,
        choices: choices.iter().map(|choice| (*choice).to_string()).collect(),
    }
}

pub(crate) fn event(payload: &[(&str, ComponentPropertyKind)]) -> ComponentEventSpec {
    ComponentEventSpec {
        payload: payload
            .iter()
            .map(|(name, kind)| ((*name).to_string(), *kind))
            .collect(),
    }
}

pub(crate) fn events(
    values: &[(&str, ComponentEventSpec)],
) -> BTreeMap<String, ComponentEventSpec> {
    values
        .iter()
        .map(|(name, event)| ((*name).to_string(), event.clone()))
        .collect()
}
