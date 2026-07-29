use dioxus::prelude::*;
use rudi::Singleton;

use crate::component::{
    ComponentDefinition, ComponentRenderContext, ComponentShape, DynamicComponentProvider, bind_dynamic_component, spec,
};

/// 表格标题单元格。
#[derive(Clone, Debug)]
#[Singleton(name = module_path!(), binds = [bind_dynamic_component::<Th>])]
pub struct Th;

impl DynamicComponentProvider for Th {
    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Leaf,
            spec: spec(
                "th",
                "remote-ui-table-head h-10 px-4 text-left align-middle font-medium text-muted-foreground whitespace-nowrap",
            ),
        }
    }


    fn render(&self, context: ComponentRenderContext) -> Element {
        let text = context.body_text();
        rsx! { th { class: "h-10 whitespace-nowrap px-4 text-left align-middle font-medium text-muted-foreground", "{text}" } }
    }
}
