use std::{collections::BTreeMap, sync::Arc};

use crate::{PropertyValue, RenderNode, RenderPlan, SymbolId};
use anyhow::{Context, Result, bail};
use dioxus::prelude::*;
use serde_json::Value;

use crate::{ComponentIndex, ComponentRenderContext, DynamicComponentEvent};

/// 一次页面渲染固定持有的数据快照。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DynamicRenderData {
    pub page_state: BTreeMap<SymbolId, Value>,
    pub data_sources: BTreeMap<SymbolId, Value>,
    pub event_payload: BTreeMap<String, Value>,
}

/// 只负责递归、绑定与 Provider 调度的 Dioxus Renderer。
#[derive(Clone)]
pub struct DynamicRenderer {
    components: Arc<ComponentIndex>,
}

impl DynamicRenderer {
    #[must_use]
    pub fn new(components: Arc<ComponentIndex>) -> Self {
        Self { components }
    }

    pub fn render(
        &self,
        plan: &RenderPlan,
        data: &DynamicRenderData,
        dispatch: Callback<DynamicComponentEvent>,
    ) -> Element {
        self.render_node(&plan.root, data, dispatch)
    }

    fn render_node(
        &self,
        node: &RenderNode,
        data: &DynamicRenderData,
        dispatch: Callback<DynamicComponentEvent>,
    ) -> Element {
        let component = match self.components.resolve_canonical(&node.component) {
            Ok(value) => value,
            Err(error) => return render_error(node.id, &error.to_string()),
        };
        let properties = match node
            .properties
            .iter()
            .map(|(name, value)| {
                resolve_property(value, data)
                    .map(|resolved| (name.clone(), resolved))
                    .with_context(|| format!("解析组件属性失败: {name}"))
            })
            .collect::<Result<BTreeMap<_, _>>>()
        {
            Ok(value) => value,
            Err(error) => return render_error(node.id, &error.to_string()),
        };
        let content = match node
            .content
            .as_ref()
            .map(|value| resolve_property(value, data).and_then(value_to_text))
            .transpose()
        {
            Ok(value) => value,
            Err(error) => return render_error(node.id, &error.to_string()),
        };
        let children = node
            .children
            .iter()
            .map(|child| self.render_node(child, data, dispatch))
            .collect::<Vec<_>>();
        let children = rsx! {
            for child in children {
                {child}
            }
        };
        component.render(ComponentRenderContext {
            component_id: node.id,
            properties,
            content,
            children,
            dispatch,
            style: node.style.clone(),
        })
    }
}

fn resolve_property(value: &PropertyValue, data: &DynamicRenderData) -> Result<Value> {
    match value {
        PropertyValue::Literal { value } => Ok(value.clone()),
        PropertyValue::PageState { state_id } => data
            .page_state
            .get(state_id)
            .cloned()
            .with_context(|| format!("页面状态不存在: {state_id}")),
        PropertyValue::DataSource { source_id, path } => {
            let mut value = data
                .data_sources
                .get(source_id)
                .with_context(|| format!("数据源不存在: {source_id}"))?;
            for field_id in path {
                value = value
                    .get(field_id.to_string())
                    .with_context(|| format!("数据源字段不存在: {field_id}"))?;
            }
            Ok(value.clone())
        }
        PropertyValue::EventValue { name } => data
            .event_payload
            .get(name)
            .cloned()
            .with_context(|| format!("事件字段不存在: {name}")),
    }
}

fn value_to_text(value: Value) -> Result<String> {
    match value {
        Value::String(value) => Ok(value),
        Value::Number(value) => Ok(value.to_string()),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Null => Ok(String::new()),
        Value::Array(_) | Value::Object(_) => bail!("组件正文只接受标量值"),
    }
}

fn render_error(component_id: SymbolId, message: &str) -> Element {
    rsx! {
        div {
            class: "rounded-md border border-destructive bg-destructive/10 p-3 text-sm text-destructive break-words",
            "data-component-id": component_id.to_string(),
            "{message}"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PropertyValue;
    use serde_json::json;

    #[test]
    fn resolves_state_and_stable_field_path() -> Result<()> {
        let source_id = SymbolId::new();
        let field_id = SymbolId::new();
        let data = DynamicRenderData {
            data_sources: BTreeMap::from([(source_id, json!({field_id.to_string(): "ready"}))]),
            ..DynamicRenderData::default()
        };
        let value = resolve_property(
            &PropertyValue::DataSource {
                source_id,
                path: vec![field_id],
            },
            &data,
        )?;
        assert_eq!(value, json!("ready"));
        Ok(())
    }
}
