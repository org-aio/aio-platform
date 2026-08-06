use crate::WorkbenchBootstrap;

pub fn load_from_document() -> WorkbenchBootstrap {
    let json = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("aio-bootstrap"))
        .and_then(|element| element.text_content())
        .unwrap_or_default();
    serde_json::from_str(&json).unwrap_or_default()
}

pub fn page_title(bootstrap: &WorkbenchBootstrap, route: &str) -> String {
    if route == "/studio" {
        return "Studio".to_owned();
    }
    bootstrap
        .native_entries
        .iter()
        .find(|entry| entry.route == route)
        .map(|entry| entry.title.clone())
        .or_else(|| bootstrap.route(route).map(|(_, route)| route.name.clone()))
        .unwrap_or_else(|| "AIO".to_owned())
}
