use dioxus::prelude::*;
use rudi::Singleton;

use crate::component::{
    ComponentDefinition, ComponentRenderContext, ComponentShape, DynamicComponentProvider, bind_dynamic_component, spec,
};

/// 普通说明段落。
#[derive(Clone, Debug)]
#[Singleton(name = module_path!(), binds = [bind_dynamic_component::<P>])]
pub struct P;

impl DynamicComponentProvider for P {
    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Leaf,
            spec: spec(
                "p",
                "remote-ui-paragraph text-sm text-muted-foreground",
            ),
        }
    }


    fn render(&self, context: ComponentRenderContext) -> Element {
        let text = context.body_text();
        rsx! { p { class: "text-sm text-muted-foreground break-words", "{text}" } }
    }
}
