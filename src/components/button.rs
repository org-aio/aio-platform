use dioxus::prelude::*;
use rudi::Singleton;

use crate::component::{
    ComponentBehavior, ComponentDefinition, ComponentRenderContext, ComponentPropertyKind, ComponentShape,
    DynamicComponentProvider, bind_dynamic_component, choice_property, event, events, properties, property,
    spec_with_variants,
};

/// 通过 action ID 上报交互的按钮。
#[derive(Clone, Debug)]
#[Singleton(name = module_path!(), binds = [bind_dynamic_component::<Button>])]
pub struct Button;

impl DynamicComponentProvider for Button {
    fn semantic_names(&self) -> Vec<String> {
        vec!["按钮".to_string(), "操作".to_string()]
    }

    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Leaf,
            spec: spec_with_variants(
                "button",
                "remote-ui-button inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium transition-all shrink-0 w-fit h-9 px-4 py-2",
                &[
                    (
                        "primary",
                        "bg-primary text-primary-foreground shadow-xs hover:bg-primary/90",
                    ),
                    (
                        "destructive",
                        "bg-destructive text-white shadow-xs hover:bg-destructive/90",
                    ),
                    (
                        "outline",
                        "border bg-background shadow-xs hover:bg-accent hover:text-accent-foreground",
                    ),
                    ("ghost", "hover:bg-accent hover:text-accent-foreground"),
                ],
                "primary",
                ComponentBehavior::Button,
            ),
        }
    }

    fn render(&self, context: ComponentRenderContext) -> Element {
        let text = if context.body_text().is_empty() {
            context.text("tx")
        } else {
            context.body_text()
        };
        let variant = context.text("v");
        let variant_class = match variant.as_str() {
            "destructive" => "bg-destructive text-white hover:bg-destructive/90",
            "outline" => "border bg-background hover:bg-accent",
            "ghost" => "hover:bg-accent",
            _ => "bg-primary text-primary-foreground hover:bg-primary/90",
        };
        let disabled = context.boolean("disabled");
        let action = context.text("act");
        let component_id = context.component_id;
        let dispatch = context.dispatch;
        rsx! {
            button {
                class: "inline-flex h-9 w-fit shrink-0 items-center justify-center gap-2 rounded-md px-4 py-2 text-sm font-medium transition-colors {variant_class}",
                disabled,
                onclick: move |_| dispatch.call(crate::DynamicComponentEvent {
                    component_id,
                    event: "click".to_owned(),
                    payload: std::collections::BTreeMap::from([(
                        "action".to_owned(),
                        serde_json::Value::String(action.clone()),
                    )]),
                }),
                "{text}"
            }
        }
    }

    fn properties(&self) -> std::collections::BTreeMap<String, crate::ComponentPropertySpec> {
        properties(&[
            ("tx", property(ComponentPropertyKind::Text, false)),
            ("act", property(ComponentPropertyKind::Action, false)),
            (
                "v",
                choice_property(&["primary", "destructive", "outline", "ghost"]),
            ),
            (
                "disabled",
                property(ComponentPropertyKind::Boolean, false),
            ),
        ])
    }

    fn events(&self) -> std::collections::BTreeMap<String, crate::ComponentEventSpec> {
        events(&[(
            "click",
            event(&[("action", ComponentPropertyKind::Action)]),
        )])
    }
}
