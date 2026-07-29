use dioxus::prelude::*;
use rudi::Singleton;

use crate::component::{
    ComponentDefinition, ComponentRenderContext, ComponentShape, DynamicComponentProvider, bind_dynamic_component, spec,
};

/// 页面内的顶层内容分区。
#[derive(Clone, Debug)]
#[Singleton(name = module_path!(), binds = [bind_dynamic_component::<Section>])]
pub struct Section;

impl DynamicComponentProvider for Section {
    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Container,
            spec: spec("section", "remote-ui-section space-y-6"),
        }
    }


    fn render(&self, context: ComponentRenderContext) -> Element {
        let children = context.children;
        rsx! { section { class: "space-y-6", {children} } }
    }
}
