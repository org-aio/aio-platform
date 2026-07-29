use dioxus::prelude::*;
use rudi::Singleton;

use crate::component::{
    ComponentDefinition, ComponentRenderContext, ComponentShape, DynamicComponentProvider, bind_dynamic_component, spec,
};

/// 卡片标题区域。
#[derive(Clone, Debug)]
#[Singleton(name = module_path!(), binds = [bind_dynamic_component::<CardHeader>])]
pub struct CardHeader;

impl DynamicComponentProvider for CardHeader {
    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Container,
            spec: spec(
                "header",
                "remote-ui-card-header grid auto-rows-min grid-rows-[auto_auto] items-start gap-1.5 px-6",
            ),
        }
    }


    fn render(&self, context: ComponentRenderContext) -> Element {
        let children = context.children;
        rsx! { header { class: "grid auto-rows-min items-start gap-1.5 px-6", {children} } }
    }
}
