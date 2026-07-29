use dioxus::prelude::*;
use rudi::Singleton;

use crate::component::{
    ComponentDefinition, ComponentRenderContext, ComponentShape, DynamicComponentProvider, bind_dynamic_component, spec,
};

/// 保留流式正文空格的代码区域。
#[derive(Clone, Debug)]
#[Singleton(name = module_path!(), binds = [bind_dynamic_component::<Code>])]
pub struct Code;

impl DynamicComponentProvider for Code {
    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Container,
            spec: spec(
                "pre",
                "remote-ui-code overflow-auto rounded-md bg-muted p-4 text-sm font-mono",
            ),
        }
    }


    fn render(&self, context: ComponentRenderContext) -> Element {
        let text = context.body_text();
        let children = context.children;
        rsx! {
            pre { class: "overflow-auto rounded-md bg-muted p-4 font-mono text-sm whitespace-pre-wrap break-words",
                if !text.is_empty() { "{text}" }
                {children}
            }
        }
    }
}
