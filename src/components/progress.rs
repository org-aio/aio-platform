use dioxus::prelude::*;
use rudi::Singleton;

use crate::component::{
    ComponentBehavior, ComponentDefinition, ComponentRenderContext, ComponentPropertyKind, ComponentShape,
    DynamicComponentProvider, bind_dynamic_component, choice_property, properties, property, spec_with_behavior,
};

/// 支持 `[upd]` 增量更新的进度组件。
#[derive(Clone, Debug)]
#[Singleton(name = module_path!(), binds = [bind_dynamic_component::<Progress>])]
pub struct Progress;

impl DynamicComponentProvider for Progress {
    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Leaf,
            spec: spec_with_behavior(
                "div",
                "remote-ui-progress",
                ComponentBehavior::Progress,
            ),
        }
    }

    fn render(&self, context: ComponentRenderContext) -> Element {
        let label = context.text("label");
        let value = context.number("v").clamp(0.0, 100.0);
        let status = context.text("status");
        rsx! {
            div { class: "grid min-w-0 gap-1.5",
                div { class: "flex items-center justify-between gap-3 text-xs",
                    span { class: "truncate", "{label}" }
                    span { class: "shrink-0 text-muted-foreground", "{status} · {value:.0}%" }
                }
                progress { class: "h-2 w-full", max: 100, value }
            }
        }
    }

    fn properties(&self) -> std::collections::BTreeMap<String, crate::ComponentPropertySpec> {
        properties(&[
            ("label", property(ComponentPropertyKind::Text, false)),
            ("v", property(ComponentPropertyKind::Number, true)),
            (
                "status",
                choice_property(&["idle", "running", "success", "error"]),
            ),
        ])
    }
}
