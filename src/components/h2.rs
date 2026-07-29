use dioxus::prelude::*;
use rudi::Singleton;

use crate::component::{
    ComponentDefinition, ComponentRenderContext, ComponentShape, DynamicComponentProvider, bind_dynamic_component, spec,
};

/// 页面二级标题。
#[derive(Clone, Debug)]
#[Singleton(name = module_path!(), binds = [bind_dynamic_component::<H2>])]
pub struct H2;

impl DynamicComponentProvider for H2 {
    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Leaf,
            spec: spec("h2", "remote-ui-subheading text-base font-semibold"),
        }
    }


    fn render(&self, context: ComponentRenderContext) -> Element {
        let text = context.body_text();
        rsx! { h2 { class: "text-base font-semibold", "{text}" } }
    }
}
