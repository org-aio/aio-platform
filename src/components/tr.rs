use dioxus::prelude::*;
use rudi::Singleton;

use crate::component::{
    ComponentDefinition, ComponentRenderContext, ComponentShape, DynamicComponentProvider, bind_dynamic_component, spec,
};

/// 表格行。
#[derive(Clone, Debug)]
#[Singleton(name = module_path!(), binds = [bind_dynamic_component::<Tr>])]
pub struct Tr;

impl DynamicComponentProvider for Tr {
    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Container,
            spec: spec(
                "tr",
                "remote-ui-table-row border-b transition-colors hover:bg-muted/50",
            ),
        }
    }


    fn render(&self, context: ComponentRenderContext) -> Element {
        let children = context.children;
        rsx! { tr { class: "border-b transition-colors hover:bg-muted/50", {children} } }
    }
}
