use dioxus::prelude::*;

#[component]
pub(crate) fn FrontendPreload(config: String, on_prepare: Callback<String>) -> Element {
    let mut bridge = use_signal(|| None::<document::Eval>);
    use_future(move || {
        let config = config.clone();
        async move {
            let mut evaluator = document::eval(concat!(
                include_str!("frontend_cache.js"),
                "\n",
                include_str!("frontend_preload.js"),
                "\n",
                include_str!("frontend_preload_start.js")
            ));
            if evaluator.send(config).is_ok() {
                bridge.set(Some(evaluator));
                while let Ok(id) = evaluator.recv::<String>().await {
                    on_prepare.call(id);
                }
            }
        }
    });
    use_drop(move || {
        if let Some(evaluator) = bridge() {
            let _ = evaluator.send(serde_json::json!({ "dispose": true }));
        }
    });
    rsx! {}
}

#[cfg(test)]
mod tests {
    #[test]
    fn preload_waits_for_the_active_page_and_skips_it() {
        let source = include_str!("frontend_preload_start.js");
        assert!(source.contains("[data-aio-page-active=\"true\"]"));
        assert!(source.contains("iframe[data-aio-prepared=true]"));
        assert!(source.contains("warmFrontendAssets(config, controller.signal, prepare)"));
        assert!(!source.contains("}, 1500);"));
    }
}
