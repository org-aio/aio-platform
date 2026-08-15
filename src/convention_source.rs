use std::fmt::Write as _;

use crate::{
    PageDefinition, PageRendererDefinition, ProgramDefinition, convention_page_module_name,
};

pub(crate) fn convention_page_source(page: &PageDefinition) -> String {
    r#"use dioxus::prelude::*;

pub(super) fn render() -> Element {
    rsx! {
        studio::EndpointPage { page_id: "__PAGE_ID__".to_owned() }
    }
}
"#
    .replace("__PAGE_ID__", &page.id.to_string())
}

pub(crate) fn convention_pages_source(program: &ProgramDefinition) -> String {
    let pages = program
        .pages
        .iter()
        .filter(|page| matches!(page.renderer, PageRendererDefinition::ConventionFile))
        .collect::<Vec<_>>();
    let mut source = String::from("use dioxus::prelude::*;\nuse studio::SymbolId;\n\n");
    let mut module_names = pages
        .iter()
        .map(|page| convention_page_module_name(&program.name, &page.name))
        .collect::<Vec<_>>();
    module_names.sort();
    for module_name in module_names {
        let _ = writeln!(source, "mod {module_name};");
    }
    source.push_str("\npub fn render(page_id: SymbolId) -> Element {\n");
    source.push_str("    match page_id.to_string().as_str() {\n");
    for page in pages {
        let _ = writeln!(
            source,
            "        {:?} => {}::render(),",
            page.id.to_string(),
            convention_page_module_name(&program.name, &page.name)
        );
    }
    source
        .push_str("        _ => rsx! { studio::EndpointPage { page_id: page_id.to_string() } },\n");
    source.push_str("    }\n}\n");
    source
}

#[cfg(test)]
mod tests {
    use crate::{DefinitionState, SymbolId};

    use super::*;

    fn convention_page(name: &str) -> PageDefinition {
        PageDefinition {
            id: SymbolId::new(),
            name: name.to_owned(),
            title: name.to_owned(),
            state: DefinitionState::Known,
            renderer: PageRendererDefinition::ConventionFile,
            endpoints: Vec::new(),
        }
    }

    #[test]
    fn generated_page_has_no_plugin_identity() {
        let source = convention_page_source(&convention_page("orders"));

        assert!(source.contains("pub(super) fn render()"));
        assert!(!source.contains("module_path!()"));
        assert!(!source.contains("Provider"));
        assert!(source.contains("studio::EndpointPage"));
    }

    #[test]
    fn generated_dispatcher_orders_modules() {
        let mut program = ProgramDefinition::empty("example-app", "示例应用");
        program.pages = vec![convention_page("users"), convention_page("orders")];

        let source = convention_pages_source(&program);
        let orders = source.find("mod example_app_orders;").unwrap_or(usize::MAX);
        let users = source.find("mod example_app_users;").unwrap_or(usize::MAX);

        assert!(orders < users);
    }
}
