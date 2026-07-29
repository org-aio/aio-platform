use dioxus::prelude::*;
use rudi::Singleton;

use crate::component::{
    ComponentBehavior, ComponentDefinition, ComponentRenderContext, ComponentPropertyKind, ComponentShape,
    DynamicComponentProvider, bind_dynamic_component, choice_property, properties, property, spec_with_variants,
};

/// 表达状态和分类的徽标。
#[derive(Clone, Debug)]
#[Singleton(name = module_path!(), binds = [bind_dynamic_component::<Badge>])]
pub struct Badge;

impl DynamicComponentProvider for Badge {
    fn definition(&self) -> ComponentDefinition {
        ComponentDefinition {
            shape: ComponentShape::Leaf,
            spec: spec_with_variants(
                "span",
                "remote-ui-badge inline-flex items-center font-semibold rounded-md border transition-colors w-fit px-2 py-1 text-xs",
                &[
                    ("default", "bg-primary text-primary-foreground"),
                    ("outline", "text-foreground"),
                    ("success", "remote-ui-badge-success"),
                    ("warning", "remote-ui-badge-warning"),
                    ("danger", "bg-destructive text-destructive-foreground"),
                ],
                "default",
                ComponentBehavior::Generic,
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
            "outline" => "text-foreground",
            "success" => "bg-emerald-100 text-emerald-800 border-emerald-300",
            "warning" => "bg-amber-100 text-amber-900 border-amber-300",
            "danger" => "bg-destructive text-destructive-foreground",
            _ => "bg-primary text-primary-foreground",
        };
        rsx! {
            span { class: "inline-flex w-fit items-center rounded-md border px-2 py-1 text-xs font-semibold {variant_class}", "{text}" }
        }
    }

    fn properties(&self) -> std::collections::BTreeMap<String, crate::ComponentPropertySpec> {
        properties(&[
            ("tx", property(ComponentPropertyKind::Text, false)),
            (
                "v",
                choice_property(&["default", "outline", "success", "warning", "danger"]),
            ),
        ])
    }
}
