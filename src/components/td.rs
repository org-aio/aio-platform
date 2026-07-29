use dioxus::prelude::*;
use rudi::Singleton;

use crate::component::{
    ComponentDefinition, ComponentRenderContext, ComponentShape, DynamicComponentProvider, bind_dynamic_component, spec,
};

/// 可直接放文本或嵌套状态组件的表格单元格。
#[derive(Clone, Debug)]
#[Singleton(name = module_path!(), binds = [bind_dynamic_component::<Td>])]
pub struct Td;

impl DynamicComponentProvider for Td {
    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Dual,
            spec: spec("td", "remote-ui-table-cell p-4 align-middle"),
        }
    }


    fn render(&self, context: ComponentRenderContext) -> Element {
        let text = context.body_text();
        let children = context.children;
        rsx! { td { class: "p-4 align-middle break-words", if !text.is_empty() { "{text}" } {children} } }
    }
}
