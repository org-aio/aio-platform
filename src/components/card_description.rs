use dioxus::prelude::*;
use rudi::Singleton;

use crate::component::{
    ComponentDefinition, ComponentRenderContext, ComponentShape, DynamicComponentProvider, bind_dynamic_component, spec,
};

/// 卡片辅助说明。
#[derive(Clone, Debug)]
#[Singleton(name = module_path!(), binds = [bind_dynamic_component::<CardDescription>])]
pub struct CardDescription;

impl DynamicComponentProvider for CardDescription {
    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Leaf,
            spec: spec(
                "p",
                "remote-ui-card-description text-muted-foreground text-sm",
            ),
        }
    }


    fn render(&self, context: ComponentRenderContext) -> Element {
        let text = context.body_text();
        rsx! { p { class: "text-sm text-muted-foreground", "{text}" } }
    }
}
