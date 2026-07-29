use dioxus::prelude::*;
use rudi::Singleton;

use crate::component::{
    ComponentBehavior, ComponentDefinition, ComponentRenderContext, ComponentPropertyKind, ComponentShape,
    DynamicComponentProvider, bind_dynamic_component, properties, property, spec_with_behavior,
};

/// 带滚动边界的数据表格。
#[derive(Clone, Debug)]
#[Singleton(name = module_path!(), binds = [bind_dynamic_component::<Table>])]
pub struct Table;

impl DynamicComponentProvider for Table {
    fn semantic_names(&self) -> Vec<String> {
        vec!["表格".to_string(), "列表".to_string()]
    }

    fn render(&self, context: ComponentRenderContext) -> Element {
        let children = context.children;
        rsx! {
            div { class: "w-full overflow-auto",
                table { class: "w-full caption-bottom text-sm", {children} }
            }
        }
    }

    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Container,
            spec: spec_with_behavior(
                "table",
                "remote-ui-table w-full text-sm caption-bottom",
                ComponentBehavior::Table,
            ),
        }
    }

    fn properties(&self) -> std::collections::BTreeMap<String, crate::ComponentPropertySpec> {
        properties(&[
            ("source", property(ComponentPropertyKind::Text, false)),
            ("columns", property(ComponentPropertyKind::Text, false)),
        ])
    }
}
