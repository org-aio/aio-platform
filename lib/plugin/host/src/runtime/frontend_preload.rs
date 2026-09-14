use dioxus::prelude::*;

#[component]
pub(crate) fn FrontendPreload(config: String, on_prepare: Callback<String>) -> Element {
    let mut bridge = use_signal(|| None::<document::Eval>);
    use_future(move || {
        let config = config.clone();
        async move {
            let mut evaluator = document::eval(concat!(
                include_str!("frontend_cache.js"), "\n",
                include_str!("frontend_preload.js"), "\n",
                "const config = JSON.parse(await dioxus.recv());
                 const controller = new AbortController();
                 const leave = () => controller.abort();
                 window.addEventListener('pagehide', leave);
                 const prepare = async id => {
                   dioxus.send(id);
                   const deadline = Date.now() + 30000;
                   while (!controller.signal.aborted && Date.now() < deadline) {
                     const page = [...document.querySelectorAll('[data-aio-page]')].find(node => node.dataset.aioPage === id && node.dataset.aioWorkspaceContext === config.context);
                     if (page?.querySelector('iframe[data-aio-prepared=true]')) return true;
                     await new Promise(resolve => setTimeout(resolve, 250));
                   }
                   return false;
                 };
                 const timer = setTimeout(() => { void warmFrontendAssets(config, controller.signal, prepare).catch(() => {}); }, 1500);
                 try { await dioxus.recv(); } finally {
                   clearTimeout(timer); controller.abort();
                   window.removeEventListener('pagehide', leave);
                 }"
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
