use dioxus::prelude::*;
use rudi::Singleton;

use crate::component::{
    ComponentDefinition, ComponentRenderContext, ComponentShape, DynamicComponentProvider, bind_dynamic_component, spec,
};

/// 表格数据行组。
#[derive(Clone, Debug)]
#[Singleton(name = module_path!(), binds = [bind_dynamic_component::<Tbody>])]
pub struct Tbody;

impl DynamicComponentProvider for Tbody {
    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Container,
            spec: spec("tbody", "remote-ui-table-body"),
        }
    }


    fn render(&self, context: ComponentRenderContext) -> Element {
        let children = context.children;
        rsx! { tbody { {children} } }
    }
}
