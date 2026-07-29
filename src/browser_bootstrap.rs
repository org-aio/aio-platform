use crate::WorkbenchBootstrap;
use wasm_bindgen::JsValue;

pub fn load_from_document() -> WorkbenchBootstrap {
    let json = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("aio-bootstrap"))
        .and_then(|element| element.text_content())
        .unwrap_or_default();
    serde_json::from_str(&json).unwrap_or_default()
}

pub fn initial_route(bootstrap: &WorkbenchBootstrap) -> String {
    web_sys::window()
        .and_then(|window| window.location().search().ok())
        .and_then(|search| route_from_search(&search))
        .filter(|route| !route.is_empty())
        .unwrap_or_else(|| bootstrap.default_route.clone())
}

pub fn page_title(bootstrap: &WorkbenchBootstrap, route: &str) -> String {
    bootstrap
        .native_entries
        .iter()
        .find(|entry| entry.route == route)
        .map(|entry| entry.title.clone())
        .or_else(|| {
            bootstrap
                .route(route)
                .map(|(application, route)| format!("{} · {}", application.title, route.name))
        })
        .unwrap_or_else(|| "AIO".to_owned())
}

pub fn push_route(route: &str) {
    if let Some(window) = web_sys::window()
        && let Ok(history) = window.history()
    {
        let _ = history.push_state_with_url(&JsValue::NULL, "", Some(&format!("?route={route}")));
    }
}

fn route_from_search(search: &str) -> Option<String> {
    web_sys::UrlSearchParams::new_with_str(search)
        .ok()?
        .get("route")
}
