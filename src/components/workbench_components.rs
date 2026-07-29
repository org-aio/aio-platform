use std::collections::BTreeMap;

use dioxus::html::FormValue;
use dioxus::prelude::*;
use rudi::Singleton;
use serde_json::{Value, json};

use crate::component::{
    ComponentDefinition, ComponentPropertyKind, ComponentRenderContext, ComponentShape,
    DynamicComponentProvider, bind_dynamic_component, choice_property, event, events, properties,
    property, spec,
};
use crate::DynamicComponentEvent;

macro_rules! container_provider {
    ($name:ident, $element:ident, $class:literal, $native_name:literal) => {
        #[derive(Clone, Debug)]
        #[Singleton(
            name = concat!(module_path!(), "::", stringify!($name)),
            binds = [bind_dynamic_component::<$name>]
        )]
        pub struct $name;

        impl DynamicComponentProvider for $name {
            fn semantic_names(&self) -> Vec<String> {
                vec![$native_name.to_owned()]
            }

            fn definition(&self) -> ComponentDefinition {
                ComponentDefinition {
                    shape: ComponentShape::Container,
                    spec: spec(stringify!($element), $class),
                }
            }

            fn render(&self, context: ComponentRenderContext) -> Element {
                let layout = context.layout_class();
                let children = context.children;
                let class = format!("{} {layout}", $class);
                rsx! { $element { class, {children} } }
            }
        }
    };
}

container_provider!(Stack, div, "flex min-w-0 flex-col gap-4", "纵向布局");
container_provider!(Grid, div, "grid min-w-0 grid-cols-1 gap-4", "栅格布局");
container_provider!(List, ul, "grid min-w-0 gap-2", "列表");
container_provider!(Tree, div, "grid min-w-0 gap-1", "树");

#[derive(Clone, Debug)]
#[Singleton(
    name = concat!(module_path!(), "::Alert"),
    binds = [bind_dynamic_component::<Alert>]
)]
pub struct Alert;

impl DynamicComponentProvider for Alert {
    fn semantic_names(&self) -> Vec<String> {
        vec!["提示".to_owned(), "告警".to_owned()]
    }

    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Dual,
            spec: spec("div", "rounded-md border p-4"),
        }
    }

    fn render(&self, context: ComponentRenderContext) -> Element {
        let title = context.text("title");
        let message = if context.body_text().is_empty() {
            context.text("message")
        } else {
            context.body_text()
        };
        let variant = context.text("variant");
        let variant_class = match variant.as_str() {
            "error" => "border-destructive bg-destructive/10 text-destructive",
            "warning" => "border-amber-400 bg-amber-50 text-amber-950",
            "success" => "border-emerald-400 bg-emerald-50 text-emerald-950",
            _ => "border-border bg-muted/50 text-foreground",
        };
        let children = context.children;
        rsx! {
            div { role: "alert", class: "rounded-md border p-4 {variant_class}",
                if !title.is_empty() { div { class: "text-sm font-semibold", "{title}" } }
                if !message.is_empty() { p { class: "mt-1 text-sm break-words", "{message}" } }
                {children}
            }
        }
    }

    fn properties(&self) -> BTreeMap<String, crate::ComponentPropertySpec> {
        properties(&[
            ("title", property(ComponentPropertyKind::Text, false)),
            ("message", property(ComponentPropertyKind::Text, false)),
            (
                "variant",
                choice_property(&["info", "success", "warning", "error"]),
            ),
        ])
    }
}

#[derive(Clone, Debug)]
#[Singleton(
    name = concat!(module_path!(), "::Empty"),
    binds = [bind_dynamic_component::<Empty>]
)]
pub struct Empty;

impl DynamicComponentProvider for Empty {
    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Dual,
            spec: spec("div", "grid place-items-center border border-dashed p-8"),
        }
    }

    fn render(&self, context: ComponentRenderContext) -> Element {
        let title = context.text("title");
        let message = context.text("message");
        let children = context.children;
        rsx! {
            div { class: "grid min-h-40 place-items-center rounded-md border border-dashed p-8 text-center",
                div { class: "grid max-w-md gap-2",
                    strong { class: "text-sm", "{title}" }
                    p { class: "text-sm text-muted-foreground break-words", "{message}" }
                    {children}
                }
            }
        }
    }

    fn properties(&self) -> BTreeMap<String, crate::ComponentPropertySpec> {
        properties(&[
            ("title", property(ComponentPropertyKind::Text, false)),
            ("message", property(ComponentPropertyKind::Text, false)),
        ])
    }
}

#[derive(Clone, Debug)]
#[Singleton(
    name = concat!(module_path!(), "::Textarea"),
    binds = [bind_dynamic_component::<Textarea>]
)]
pub struct Textarea;

impl DynamicComponentProvider for Textarea {
    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Leaf,
            spec: spec("textarea", "min-h-24 rounded-md border px-3 py-2"),
        }
    }

    fn render(&self, context: ComponentRenderContext) -> Element {
        let label = context.text("label");
        let name = context.text("name");
        let value = context.text("value");
        let placeholder = context.text("placeholder");
        let required = context.boolean("required");
        let component_id = context.component_id;
        let dispatch = context.dispatch;
        rsx! {
            label { class: "grid min-w-0 gap-1.5 text-sm",
                if !label.is_empty() { span { class: "font-medium", "{label}" } }
                textarea {
                    class: "min-h-24 min-w-0 resize-y rounded-md border bg-background px-3 py-2 text-sm text-foreground",
                    name: name.clone(),
                    value,
                    placeholder,
                    required,
                    oninput: move |event| dispatch_value(
                        dispatch,
                        component_id,
                        "change",
                        &name,
                        Value::String(event.value()),
                    ),
                }
            }
        }
    }

    fn properties(&self) -> BTreeMap<String, crate::ComponentPropertySpec> {
        properties(&[
            ("label", property(ComponentPropertyKind::Text, false)),
            ("name", property(ComponentPropertyKind::Text, false)),
            ("value", property(ComponentPropertyKind::Text, false)),
            ("placeholder", property(ComponentPropertyKind::Text, false)),
            ("required", property(ComponentPropertyKind::Boolean, false)),
        ])
    }

    fn events(&self) -> BTreeMap<String, crate::ComponentEventSpec> {
        events(&[(
            "change",
            event(&[
                ("name", ComponentPropertyKind::Text),
                ("value", ComponentPropertyKind::Text),
            ]),
        )])
    }
}

#[derive(Clone, Debug)]
#[Singleton(
    name = concat!(module_path!(), "::Select"),
    binds = [bind_dynamic_component::<Select>]
)]
pub struct Select;

impl DynamicComponentProvider for Select {
    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Leaf,
            spec: spec("select", "h-9 rounded-md border px-3"),
        }
    }

    fn render(&self, context: ComponentRenderContext) -> Element {
        let label = context.text("label");
        let name = context.text("name");
        let selected = context.text("value");
        let options = context
            .properties
            .get("options")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let component_id = context.component_id;
        let dispatch = context.dispatch;
        rsx! {
            label { class: "grid min-w-0 gap-1.5 text-sm",
                if !label.is_empty() { span { class: "font-medium", "{label}" } }
                select {
                    class: "h-9 min-w-0 rounded-md border bg-background px-3 text-sm text-foreground",
                    name: name.clone(),
                    value: selected,
                    onchange: move |event| dispatch_value(
                        dispatch,
                        component_id,
                        "change",
                        &name,
                        Value::String(event.value()),
                    ),
                    for option in options {
                        {
                            let value = option.get("value").and_then(Value::as_str).unwrap_or_default();
                            let label = option.get("label").and_then(Value::as_str).unwrap_or(value);
                            rsx! { option { value, "{label}" } }
                        }
                    }
                }
            }
        }
    }

    fn properties(&self) -> BTreeMap<String, crate::ComponentPropertySpec> {
        properties(&[
            ("label", property(ComponentPropertyKind::Text, false)),
            ("name", property(ComponentPropertyKind::Text, false)),
            ("value", property(ComponentPropertyKind::Text, false)),
            ("options", property(ComponentPropertyKind::Json, true)),
        ])
    }

    fn events(&self) -> BTreeMap<String, crate::ComponentEventSpec> {
        events(&[(
            "change",
            event(&[
                ("name", ComponentPropertyKind::Text),
                ("value", ComponentPropertyKind::Text),
            ]),
        )])
    }
}

#[derive(Clone, Debug)]
#[Singleton(
    name = concat!(module_path!(), "::Checkbox"),
    binds = [bind_dynamic_component::<Checkbox>]
)]
pub struct Checkbox;

impl DynamicComponentProvider for Checkbox {
    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Leaf,
            spec: spec("input", "size-4 rounded border"),
        }
    }

    fn render(&self, context: ComponentRenderContext) -> Element {
        let label = context.text("label");
        let name = context.text("name");
        let checked = context.boolean("checked");
        let component_id = context.component_id;
        let dispatch = context.dispatch;
        rsx! {
            label { class: "inline-flex min-w-0 items-center gap-2 text-sm",
                input {
                    r#type: "checkbox",
                    class: "size-4 rounded border",
                    name: name.clone(),
                    checked,
                    onchange: move |event| dispatch_value(
                        dispatch,
                        component_id,
                        "change",
                        &name,
                        Value::Bool(event.checked()),
                    ),
                }
                span { class: "break-words", "{label}" }
            }
        }
    }

    fn properties(&self) -> BTreeMap<String, crate::ComponentPropertySpec> {
        properties(&[
            ("label", property(ComponentPropertyKind::Text, false)),
            ("name", property(ComponentPropertyKind::Text, false)),
            ("checked", property(ComponentPropertyKind::Boolean, false)),
        ])
    }

    fn events(&self) -> BTreeMap<String, crate::ComponentEventSpec> {
        events(&[(
            "change",
            event(&[
                ("name", ComponentPropertyKind::Text),
                ("value", ComponentPropertyKind::Boolean),
            ]),
        )])
    }
}

#[derive(Clone, Debug)]
#[Singleton(
    name = concat!(module_path!(), "::Form"),
    binds = [bind_dynamic_component::<Form>]
)]
pub struct Form;

impl DynamicComponentProvider for Form {
    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Container,
            spec: spec("form", "grid gap-4"),
        }
    }

    fn render(&self, context: ComponentRenderContext) -> Element {
        let component_id = context.component_id;
        let dispatch = context.dispatch;
        let children = context.children;
        rsx! {
            form {
                class: "grid min-w-0 gap-4",
                onsubmit: move |event| {
                    event.prevent_default();
                    let values = event
                        .values()
                        .into_iter()
                        .map(|(name, value)| {
                            let value = match value {
                                FormValue::Text(value) => Value::String(value),
                                FormValue::File(file) => file
                                    .map(|file| json!({"name": file.name(), "size": file.size()}))
                                    .unwrap_or(Value::Null),
                            };
                            (name, value)
                        })
                        .collect();
                    dispatch.call(DynamicComponentEvent {
                        component_id,
                        event: "submit".to_owned(),
                        payload: values,
                    });
                },
                {children}
            }
        }
    }

    fn events(&self) -> BTreeMap<String, crate::ComponentEventSpec> {
        events(&[(
            "submit",
            event(&[("values", ComponentPropertyKind::Json)]),
        )])
    }
}

#[derive(Clone, Debug)]
#[Singleton(
    name = concat!(module_path!(), "::Dialog"),
    binds = [bind_dynamic_component::<Dialog>]
)]
pub struct Dialog;

impl DynamicComponentProvider for Dialog {
    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Container,
            spec: spec("dialog", "rounded-lg border bg-background p-0 shadow-lg"),
        }
    }

    fn render(&self, context: ComponentRenderContext) -> Element {
        let open = context.boolean("open");
        let title = context.text("title");
        let component_id = context.component_id;
        let dispatch = context.dispatch;
        let children = context.children;
        rsx! {
            dialog { open, class: "w-[min(36rem,calc(100vw-2rem))] rounded-lg border bg-background p-0 shadow-lg",
                header { class: "flex items-center justify-between gap-4 border-b p-4",
                    h2 { class: "text-base font-semibold", "{title}" }
                    button {
                        r#type: "button",
                        class: "size-8 rounded-md text-xl hover:bg-accent",
                        aria_label: "关闭",
                        onclick: move |_| dispatch.call(DynamicComponentEvent {
                            component_id,
                            event: "close".to_owned(),
                            payload: BTreeMap::new(),
                        }),
                        "×"
                    }
                }
                div { class: "p-4", {children} }
            }
        }
    }

    fn properties(&self) -> BTreeMap<String, crate::ComponentPropertySpec> {
        properties(&[
            ("open", property(ComponentPropertyKind::Boolean, false)),
            ("title", property(ComponentPropertyKind::Text, false)),
        ])
    }

    fn events(&self) -> BTreeMap<String, crate::ComponentEventSpec> {
        events(&[("close", event(&[]))])
    }
}

#[derive(Clone, Debug)]
#[Singleton(
    name = concat!(module_path!(), "::Link"),
    binds = [bind_dynamic_component::<Link>]
)]
pub struct Link;

impl DynamicComponentProvider for Link {
    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Leaf,
            spec: spec("a", "text-primary underline-offset-4 hover:underline"),
        }
    }

    fn render(&self, context: ComponentRenderContext) -> Element {
        let text = if context.body_text().is_empty() {
            context.text("text")
        } else {
            context.body_text()
        };
        let component_id = context.component_id;
        let dispatch = context.dispatch;
        rsx! {
            a {
                href: "#",
                class: "text-primary underline-offset-4 hover:underline",
                onclick: move |event| {
                    event.prevent_default();
                    dispatch.call(DynamicComponentEvent {
                        component_id,
                        event: "click".to_owned(),
                        payload: BTreeMap::new(),
                    });
                },
                "{text}"
            }
        }
    }

    fn properties(&self) -> BTreeMap<String, crate::ComponentPropertySpec> {
        properties(&[("text", property(ComponentPropertyKind::Text, false))])
    }

    fn events(&self) -> BTreeMap<String, crate::ComponentEventSpec> {
        events(&[("click", event(&[]))])
    }
}

#[derive(Clone, Debug)]
#[Singleton(
    name = concat!(module_path!(), "::Log"),
    binds = [bind_dynamic_component::<Log>]
)]
pub struct Log;

impl DynamicComponentProvider for Log {
    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Leaf,
            spec: spec("pre", "overflow-auto rounded-md bg-neutral-950 p-4 text-neutral-100"),
        }
    }

    fn render(&self, context: ComponentRenderContext) -> Element {
        let text = if context.body_text().is_empty() {
            context.text("text")
        } else {
            context.body_text()
        };
        rsx! { pre { class: "max-h-96 overflow-auto rounded-md bg-neutral-950 p-4 font-mono text-xs text-neutral-100 whitespace-pre-wrap break-words", "{text}" } }
    }

    fn properties(&self) -> BTreeMap<String, crate::ComponentPropertySpec> {
        properties(&[("text", property(ComponentPropertyKind::Text, false))])
    }
}

#[derive(Clone, Debug)]
#[Singleton(
    name = concat!(module_path!(), "::FileUpload"),
    binds = [bind_dynamic_component::<FileUpload>]
)]
pub struct FileUpload;

impl DynamicComponentProvider for FileUpload {
    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Leaf,
            spec: spec("input", "rounded-md border border-dashed p-4"),
        }
    }

    fn render(&self, context: ComponentRenderContext) -> Element {
        let label = context.text("label");
        let multiple = context.boolean("multiple");
        let component_id = context.component_id;
        let dispatch = context.dispatch;
        rsx! {
            label { class: "grid gap-2 rounded-md border border-dashed p-4 text-sm",
                span { class: "font-medium", "{label}" }
                input {
                    r#type: "file",
                    multiple,
                    onchange: move |event| {
                        let files = event
                            .files()
                            .into_iter()
                            .map(|file| json!({
                                "name": file.name(),
                                "size": file.size(),
                                "content_type": file.content_type(),
                            }))
                            .collect::<Vec<_>>();
                        dispatch.call(DynamicComponentEvent {
                            component_id,
                            event: "select".to_owned(),
                            payload: BTreeMap::from([("files".to_owned(), Value::Array(files))]),
                        });
                    },
                }
            }
        }
    }

    fn properties(&self) -> BTreeMap<String, crate::ComponentPropertySpec> {
        properties(&[
            ("label", property(ComponentPropertyKind::Text, false)),
            ("multiple", property(ComponentPropertyKind::Boolean, false)),
        ])
    }

    fn events(&self) -> BTreeMap<String, crate::ComponentEventSpec> {
        events(&[(
            "select",
            event(&[("files", ComponentPropertyKind::Json)]),
        )])
    }
}

#[derive(Clone, Debug)]
#[Singleton(
    name = concat!(module_path!(), "::Loading"),
    binds = [bind_dynamic_component::<Loading>]
)]
pub struct Loading;

impl DynamicComponentProvider for Loading {
    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Leaf,
            spec: spec("div", "flex items-center gap-2 p-4"),
        }
    }

    fn render(&self, context: ComponentRenderContext) -> Element {
        let text = context.text("text");
        rsx! {
            div { class: "flex items-center gap-2 p-4 text-sm text-muted-foreground", role: "status",
                span { class: "size-4 animate-spin rounded-full border-2 border-current border-r-transparent" }
                span { "{text}" }
            }
        }
    }

    fn properties(&self) -> BTreeMap<String, crate::ComponentPropertySpec> {
        properties(&[("text", property(ComponentPropertyKind::Text, false))])
    }
}

#[derive(Clone, Debug)]
#[Singleton(
    name = concat!(module_path!(), "::ErrorState"),
    binds = [bind_dynamic_component::<ErrorState>]
)]
pub struct ErrorState;

impl DynamicComponentProvider for ErrorState {
    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Leaf,
            spec: spec("div", "rounded-md border border-destructive p-4"),
        }
    }

    fn render(&self, context: ComponentRenderContext) -> Element {
        let title = context.text("title");
        let message = context.text("message");
        rsx! {
            div { class: "rounded-md border border-destructive bg-destructive/10 p-4 text-destructive", role: "alert",
                strong { class: "text-sm", "{title}" }
                p { class: "mt-1 text-sm break-words", "{message}" }
            }
        }
    }

    fn properties(&self) -> BTreeMap<String, crate::ComponentPropertySpec> {
        properties(&[
            ("title", property(ComponentPropertyKind::Text, false)),
            ("message", property(ComponentPropertyKind::Text, false)),
        ])
    }
}

fn dispatch_value(
    dispatch: Callback<DynamicComponentEvent>,
    component_id: crate::SymbolId,
    event: &str,
    name: &str,
    value: Value,
) {
    dispatch.call(DynamicComponentEvent {
        component_id,
        event: event.to_owned(),
        payload: BTreeMap::from([
            ("name".to_owned(), Value::String(name.to_owned())),
            ("value".to_owned(), value),
        ]),
    });
}
