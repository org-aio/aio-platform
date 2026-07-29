use dioxus::prelude::*;
use rudi::Singleton;

use crate::component::{
    ComponentDefinition, ComponentRenderContext, ComponentShape, DynamicComponentProvider, bind_dynamic_component, spec,
};

/// 卡片主标题。
#[derive(Clone, Debug)]
#[Singleton(name = module_path!(), binds = [bind_dynamic_component::<CardTitle>])]
pub struct CardTitle;

impl DynamicComponentProvider for CardTitle {
    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Leaf,
            spec: spec(
                "h2",
                "remote-ui-card-title leading-none font-semibold",
            ),
        }
    }


    fn render(&self, context: ComponentRenderContext) -> Element {
        let text = context.body_text();
        rsx! { h2 { class: "text-base font-semibold leading-none", "{text}" } }
    }
}
