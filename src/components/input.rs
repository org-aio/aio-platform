use dioxus::prelude::*;
use rudi::Singleton;

use crate::component::{
    ComponentBehavior, ComponentDefinition, ComponentRenderContext, ComponentPropertyKind, ComponentShape,
    DynamicComponentProvider, bind_dynamic_component, choice_property, event, events, properties, property,
    spec_with_behavior,
};

/// 带标签和变更事件的输入框。
#[derive(Clone, Debug)]
#[Singleton(name = module_path!(), binds = [bind_dynamic_component::<Input>])]
pub struct Input;

impl DynamicComponentProvider for Input {
    fn semantic_names(&self) -> Vec<String> {
        vec!["输入框".to_string(), "表单字段".to_string()]
    }

    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Leaf,
            spec: spec_with_behavior(
                "input",
                "remote-ui-input aio-input",
                ComponentBehavior::Input,
            ),
        }
    }

    fn render(&self, context: ComponentRenderContext) -> Element {
        let label = context.text("label");
        let placeholder = context.text("ph");
        let name = context.text("name");
        let input_type = context.text("type");
        let input_type = if input_type.is_empty() {
            "text".to_owned()
        } else {
            input_type
        };
        let value = context.text("value");
        let disabled = context.boolean("disabled");
        let required = context.boolean("required");
        let readonly = context.boolean("readonly");
        let component_id = context.component_id;
        let dispatch = context.dispatch;
        rsx! {
            label { class: "grid min-w-0 gap-1.5 text-sm",
                if !label.is_empty() {
                    span { class: "font-medium", "{label}" }
                }
                input {
                    class: "h-9 min-w-0 rounded-md border bg-background px-3 py-1 text-sm text-foreground",
                    r#type: input_type,
                    name: name.clone(),
                    value,
                    placeholder,
                    disabled,
                    required,
                    readonly,
                    oninput: move |event| dispatch.call(crate::DynamicComponentEvent {
                        component_id,
                        event: "change".to_owned(),
                        payload: std::collections::BTreeMap::from([
                            ("id".to_owned(), serde_json::Value::String(name.clone())),
                            (
                                "value".to_owned(),
                                serde_json::Value::String(event.value()),
                            ),
                        ]),
                    }),
                }
            }
        }
    }

    fn properties(&self) -> std::collections::BTreeMap<String, crate::ComponentPropertySpec> {
        properties(&[
            ("label", property(ComponentPropertyKind::Text, false)),
            ("ph", property(ComponentPropertyKind::Text, false)),
            ("name", property(ComponentPropertyKind::Text, false)),
            ("value", property(ComponentPropertyKind::Text, false)),
            (
                "type",
                choice_property(&["text", "email", "number", "password", "search"]),
            ),
            (
                "disabled",
                property(ComponentPropertyKind::Boolean, false),
            ),
            (
                "required",
                property(ComponentPropertyKind::Boolean, false),
            ),
            (
                "readonly",
                property(ComponentPropertyKind::Boolean, false),
            ),
        ])
    }

    fn events(&self) -> std::collections::BTreeMap<String, crate::ComponentEventSpec> {
        events(&[(
            "change",
            event(&[
                ("id", ComponentPropertyKind::Text),
                ("value", ComponentPropertyKind::Text),
            ]),
        )])
    }
}
