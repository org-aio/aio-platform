use dioxus::prelude::*;

#[component]
pub fn DevelopmentStatus() -> Element {
    let mut status = use_signal(|| None::<az_plugin_development::DevStatus>);
    use_future(move || async move {
        let mut listener = document::eval(
            r#"
            const events = new EventSource('/api/development/events');
            let versions = '';
            events.onmessage = event => {
                const status = JSON.parse(event.data);
                if (!status) return;
                const next = JSON.stringify(status.revisions);
                if (next !== versions) {
                    versions = next;
                    window.dispatchEvent(new Event('aio:catalog-invalidated'));
                }
                dioxus.send(status);
            };
            await dioxus.recv();
            events.close();
        "#,
        );
        while let Ok(value) = listener.recv::<az_plugin_development::DevStatus>().await {
            status.set(Some(value));
        }
    });
    match status.read().as_ref() {
        Some(value) if value.phase == "failed" => {
            rsx! { p { role: "alert", "构建失败，保留上一版本：{value.message}" } }
        }
        Some(value) if value.phase == "building" => {
            rsx! { p { role: "status", "正在构建：{value.message}" } }
        }
        _ => rsx! {},
    }
}
