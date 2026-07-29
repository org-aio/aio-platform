use dioxus::prelude::*;
use rudi::Singleton;

use crate::component::{
    ComponentDefinition, ComponentRenderContext, ComponentShape, DynamicComponentProvider, bind_dynamic_component, spec,
};

/// 复用现有组件库视觉语义的卡片容器。
#[derive(Clone, Debug)]
#[Singleton(name = module_path!(), binds = [bind_dynamic_component::<Card>])]
pub struct Card;

impl DynamicComponentProvider for Card {
    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Container,
            spec: spec(
                "article",
                "remote-ui-card bg-card text-card-foreground flex flex-col gap-4 rounded-xl border py-6 shadow-sm",
            ),
        }
    }


    fn render(&self, context: ComponentRenderContext) -> Element {
        let children = context.children;
        rsx! { article { class: "flex flex-col gap-4 rounded-lg border bg-card py-6 text-card-foreground shadow-sm", {children} } }
    }
}
