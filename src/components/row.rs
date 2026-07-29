use dioxus::prelude::*;
use rudi::Singleton;

use crate::component::{
    ComponentDefinition, ComponentRenderContext, ComponentPropertyKind, ComponentShape, DynamicComponentProvider, bind_dynamic_component,
    choice_property, properties, property, spec,
};

/// 响应式组件行。
#[derive(Clone, Debug)]
#[Singleton(name = module_path!(), binds = [bind_dynamic_component::<Row>])]
pub struct Row;

impl DynamicComponentProvider for Row {
    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Container,
            spec: spec("div", "remote-ui-row grid gap-4"),
        }
    }

    fn render(&self, context: ComponentRenderContext) -> Element {
        let variant = context.text("v");
        let label = context.text("aria-label");
        let children = context.children;
        let columns = if variant == "metrics" {
            "grid-cols-1 md:grid-cols-2 xl:grid-cols-4"
        } else {
            "grid-cols-1 lg:grid-cols-[minmax(0,1fr)_20rem]"
        };
        rsx! { div { class: "grid gap-4 {columns}", aria_label: label, {children} } }
    }

    fn properties(&self) -> std::collections::BTreeMap<String, crate::ComponentPropertySpec> {
        properties(&[
            ("v", choice_property(&["metrics", "workbench"])),
            ("aria-label", property(ComponentPropertyKind::Text, false)),
        ])
    }
}
