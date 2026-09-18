use super::model::Worker;
use az_ui_components::{
    button::{Button, ButtonVariant},
    dialog::{Dialog, DialogTitle},
};
use dioxus::prelude::*;
use serde::{Serialize, de::DeserializeOwned};

fn current_origin() -> String {
    web_sys::window()
        .and_then(|window| window.location().origin().ok())
        .unwrap_or_else(|| "https://aio.addzero.site".into())
}

fn macos_command() -> String {
    format!(
        "npm install -g @zjarlin/aio-space\nbrew install restic\naio-space connect --server {}",
        current_origin()
    )
}

fn windows_command() -> String {
    format!(
        "npm install -g @zjarlin/aio-space\naio-space connect --server {} --no-browser --foreground",
        current_origin()
    )
}

async fn request<T: DeserializeOwned>(
    method: &str,
    path: &str,
    body: Option<&impl Serialize>,
) -> Result<T, String> {
    let request = gloo_net::http::RequestBuilder::new(path)
        .method(method.parse().map_err(|_| "请求方法无效")?);
    let response = match body {
        Some(body) => request.json(body).map_err(|e| e.to_string())?.send().await,
        None => request.send().await,
    }
    .map_err(|e| e.to_string())?;
    decode_response(response).await
}

pub(super) async fn decode_response<T: DeserializeOwned>(
    response: gloo_net::http::Response,
) -> Result<T, String> {
    if !response.ok() {
        return Err(format!(
            "请求失败（HTTP {}）：{}",
            response.status(),
            response.text().await.unwrap_or_default()
        ));
    }
    let response: crate::runtime::RuntimeResponse<T> =
        response.json().await.map_err(|e| e.to_string())?;
    Ok(response.data)
}

/// 登录后挂载的设备配对弹窗，授权始终使用当前 AIO 账号会话。
#[component]
pub(crate) fn WorkerPanel(pairing: Signal<Option<String>>, on_close: EventHandler<()>) -> Element {
    let mut error = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);
    let mut revoke = use_signal(|| None::<Worker>);
    let code = pairing;
    let mut notice = use_signal(|| None::<String>);
    let mut copied = use_signal(|| None::<String>);
    let macos = macos_command();
    let windows = windows_command();
    let copy_command = move |(label, command): (String, String)| {
        let label_for_state = label.clone();
        spawn(async move {
            let command = serde_json::to_string(&command).unwrap_or_else(|_| "\"\"".into());
            let script = format!(
                "try {{ await navigator.clipboard.writeText({command}); return true; }} catch (_) {{ return false; }}"
            );
            match document::eval(&script).await {
                Ok(value) if value.as_bool() == Some(true) => {
                    copied.set(Some(label_for_state));
                    let _ = document::eval(
                        "await new Promise(resolve => setTimeout(resolve, 1600)); return true;",
                    )
                    .await;
                    if copied() == Some(label) {
                        copied.set(None);
                    }
                }
                _ => error.set(Some("复制失败，请手动选择命令。".into())),
            }
        });
    };
    let mut close = move |()| match super::pairing::finish(code) {
        Ok(()) => on_close.call(()),
        Err(message) => error.set(Some(message)),
    };
    let mut workers = use_resource(move || async {
        request::<Vec<Worker>>("GET", "/api/runtime/workers", None::<&()>).await
    });
    let pending = use_resource(move || {
        let value = code();
        async move {
            let Some(value) = value else {
                return Ok(None);
            };
            let worker = super::pairing::lookup(&value).await?;
            if worker.is_none() {
                super::pairing::finish(code)?;
                notice.set(Some(super::pairing::UNAVAILABLE.into()));
            }
            Ok::<_, String>(worker)
        }
    });
    use_future(move || async move {
        loop {
            let _ = document::eval(
                "await new Promise(resolve => setTimeout(resolve, 5000)); return true;",
            )
            .await;
            workers.restart();
        }
    });
    rsx! {
        Dialog{open:true,on_open_change:move|open:bool|if !open{close(())},
            div{class:"grid gap-4",
                div{class:"flex items-center justify-between gap-2",DialogTitle{"我的设备"}Button{variant:ButtonVariant::Ghost,onclick:move |_|close(()),"关闭"}}
                section{class:"grid gap-3 rounded-md border p-3",
                    div{class:"grid gap-1",
                        h3{"配对新设备"}
                        p{class:"text-sm text-muted-foreground","在需要配对的电脑上运行对应命令，浏览器会打开 AIO 并等待你授权这台设备。"}
                    }
                    div{class:"grid gap-2",
                        div{class:"flex items-center justify-between gap-2",
                            strong{"macOS"}
                            Button{variant:ButtonVariant::Outline,onclick:{let command=macos.clone();move |_|copy_command(("macOS".into(),command.clone()))},if copied()==Some("macOS".into()){"已复制"}else{"复制命令"}}
                        }
                        pre{class:"overflow-x-auto rounded-md bg-muted p-3 text-sm",code{class:"font-mono","{macos}"}}
                    }
                    div{class:"grid gap-2",
                        div{class:"flex items-center justify-between gap-2",
                            strong{"Windows"}
                            Button{variant:ButtonVariant::Outline,onclick:{let command=windows.clone();move |_|copy_command(("Windows".into(),command.clone()))},if copied()==Some("Windows".into()){"已复制"}else{"复制命令"}}
                        }
                        pre{class:"overflow-x-auto rounded-md bg-muted p-3 text-sm",code{class:"font-mono","{windows}"}}
                        small{class:"text-sm text-muted-foreground","Windows 当前不支持后台服务，请保持该终端窗口运行。"}
                    }
                }
                if let Some(Ok(Some(worker)))=pending.read().as_ref(){
                    section{class:"grid gap-2",
                        h3{"待授权设备：{worker.label}"}
                        p{"系统：{worker.platform}"}
                        p{"允许能力：" {worker.capabilities.join("、")}}
                        Button{disabled:busy(),onclick:move |_|{let Some(value)=code() else{return;};busy.set(true);spawn(async move{match super::pairing::approve(&value).await{Ok(approved)=>{match super::pairing::finish(code){Ok(())=>{notice.set(Some(if approved{"设备已配对，后续自动连接，无需再次使用配对链接。"}else{super::pairing::UNAVAILABLE}.into()));error.set(None);},Err(e)=>error.set(Some(e))}workers.restart();},Err(e)=>error.set(Some(e))}busy.set(false);});},"授权这台设备"}
                    }
                }
                if let Some(Err(e))=pending.read().as_ref(){p{role:"alert","{e}"}}
                if let Some(message)=notice(){p{role:"status","{message}"}}
                if let Some(message)=error(){p{role:"alert","{message}"}}
                match workers.read().as_ref(){
                    Some(Ok(items))=>rsx!{
                        if items.is_empty(){p{"暂无已配对设备"}}
                        for worker in items.iter().cloned(){
                            section{key:"{worker.id}",class:"flex flex-wrap items-center justify-between gap-2 border-b pb-3",
                                div{class:"grid gap-2",
                                    strong{"{worker.label} · {worker.status}"}
                                    small{"{worker.platform}"}
                                }
                                if worker.status!="revoked"{
                                    Button{variant:ButtonVariant::Ghost,onclick:{let worker=worker.clone();move |_|revoke.set(Some(worker.clone()))},"撤销配对"}
                                }
                            }
                        }
                    },
                    Some(Err(e))=>rsx!{p{role:"alert","{e}"}},None=>rsx!{p{"正在加载设备…"}}
                }
            }
        }
        if let Some(worker)=revoke(){
            Dialog{open:true,on_open_change:move|open:bool|if !open{revoke.set(None)},DialogTitle{"撤销设备配对"}
                p{"撤销 {worker.label} 的配对后，该设备无法继续同步、执行任务或访问归档。"}
                Button{disabled:busy(),onclick:move |_|{let id=worker.id.clone();busy.set(true);spawn(async move{match request::<()>("DELETE",&format!("/api/runtime/workers/{id}"),None::<&()>).await{Ok(())=>{revoke.set(None);workers.restart();},Err(e)=>error.set(Some(e))}busy.set(false);});},"确认撤销"}
            }
        }
    }
}
