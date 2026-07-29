use dioxus::prelude::*;
use rudi::Singleton;

use crate::component::{
    ComponentDefinition, ComponentRenderContext, ComponentShape, DynamicComponentProvider, bind_dynamic_component, spec,
};

/// 页面一级标题。
#[derive(Clone, Debug)]
#[Singleton(name = module_path!(), binds = [bind_dynamic_component::<H1>])]
pub struct H1;

impl DynamicComponentProvider for H1 {
    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Leaf,
            spec: spec("h1", "remote-ui-heading text-lg font-semibold"),
        }
    }


    fn render(&self, context: ComponentRenderContext) -> Element {
        let text = context.body_text();
        rsx! { h1 { class: "text-lg font-semibold", "{text}" } }
    }
}
