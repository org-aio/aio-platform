use super::model::{Task, Worker};
use az_ui_components::{
    button::{Button, ButtonVariant},
    dialog::{Dialog, DialogTitle},
};
use dioxus::prelude::*;
use serde::{Serialize, de::DeserializeOwned};

pub(crate) async fn request<T: DeserializeOwned>(
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

/// 登录后挂载的设备管理弹窗，授权始终使用当前 AIO 账号会话。
#[component]
pub(crate) fn WorkerPanel(pairing: Option<String>, on_close: EventHandler<()>) -> Element {
    let mut error = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);
    let mut selected = use_signal(|| None::<(Worker, String)>);
    let mut revoke = use_signal(|| None::<Worker>);
    let mut code = use_signal(move || pairing);
    let mut workers = use_resource(move || async {
        request::<Vec<Worker>>("GET", "/api/runtime/workers", None::<&()>).await
    });
    let mut tasks = use_resource(move || async {
        request::<Vec<Task>>("GET", "/api/runtime/workers/tasks", None::<&()>).await
    });
    let pending = use_resource(move || {
        let code = code();
        async move {
            match code {
                Some(code) => request::<Worker>(
                    "GET",
                    &format!("/api/runtime/workers/pairings/{code}"),
                    None::<&()>,
                )
                .await
                .map(Some),
                None => Ok(None),
            }
        }
    });
    use_future(move || async move {
        loop {
            let _ = document::eval(
                "await new Promise(resolve => setTimeout(resolve, 5000)); return true;",
            )
            .await;
            workers.restart();
            tasks.restart();
        }
    });
    rsx! {
        Dialog{open:true,on_open_change:move|open:bool|if !open{on_close.call(())},
            div{class:"grid gap-4",
                div{class:"flex items-center justify-between gap-2",DialogTitle{"我的设备"}Button{variant:ButtonVariant::Ghost,onclick:move |_|on_close.call(()),"关闭"}}
                p{"登录当前账号即可管理自己的电脑和服务器。设备主动连接 AIO，无需开放本机端口。"}
                if let Some(Ok(Some(worker)))=pending.read().as_ref(){
                    section{class:"grid gap-2",
                        h3{"配对新设备：{worker.label}"}
                        p{"系统：{worker.platform}"}
                        p{"允许能力：" {worker.capabilities.join("、")}}
                        Button{disabled:busy(),onclick:move |_|{let Some(value)=code() else{return;};busy.set(true);spawn(async move{match request::<()>("POST",&format!("/api/runtime/workers/pairings/{value}"),Some(&serde_json::json!({}))).await{Ok(())=>{code.set(None);workers.restart();},Err(e)=>error.set(Some(e))}busy.set(false);});},"授权这台设备"}
                    }
                }
                if let Some(Err(e))=pending.read().as_ref(){p{role:"alert","{e}"}}
                if let Some(message)=error(){p{role:"alert","{message}"}}
                match workers.read().as_ref(){
                    Some(Ok(items))=>rsx!{
                        if items.is_empty(){p{"暂无设备。在需要管理的电脑运行 aio-space connect，浏览器会打开此处完成配对。"}}
                        for worker in items.iter().cloned(){
                            section{key:"{worker.id}",class:"grid gap-2 border-b pb-3",
                                strong{"{worker.label} · {worker.status}"}
                                small{"{worker.platform}"}
                                if worker.status!="revoked"{
                                    div{class:"flex flex-wrap gap-2",
                                        for (capability,label) in [("space.scan","扫描占用"),("space.clean-preview","预览清理"),("space.clean","清理缓存"),("space.archive","归档到 AIO"),("space.archive-list","归档列表"),("space.archive-restore","恢复归档")]{
                                            if worker.capabilities.iter().any(|v|v==capability){
                                                Button{variant:ButtonVariant::Outline,onclick:{let worker=worker.clone();move |_|selected.set(Some((worker.clone(),capability.into())))},"{label}"}
                                            }
                                        }
                                        Button{variant:ButtonVariant::Ghost,onclick:{let worker=worker.clone();move |_|revoke.set(Some(worker.clone()))},"撤销设备"}
                                    }
                                }
                            }
                        }
                    },
                    Some(Err(e))=>rsx!{p{role:"alert","{e}"}},None=>rsx!{p{"正在加载设备…"}}
                }
                h3{"最近任务"}
                if let Some(Ok(items))=tasks.read().as_ref(){
                    for task in items.iter().take(12){
                        details{key:"{task.id}",summary{"{task.capability} · {task.state}"}
                            if let Some(message)=&task.error{p{role:"alert","{message}"}}
                            if let Some(result)=&task.result{pre{class:"overflow-auto text-xs","{serde_json::to_string_pretty(result).unwrap_or_default()}"}}
                        }
                    }
                }
            }
        }
        if let Some((worker,capability))=selected(){
            super::task_form::TaskForm{worker,capability,on_close:move |_|selected.set(None),on_submitted:move |_|{selected.set(None);tasks.restart();}}
        }
        if let Some(worker)=revoke(){
            Dialog{open:true,on_open_change:move|open:bool|if !open{revoke.set(None)},DialogTitle{"撤销设备授权"}
                p{"撤销 {worker.label} 后，该设备无法继续领取任务或访问归档。"}
                Button{disabled:busy(),onclick:move |_|{let id=worker.id.clone();busy.set(true);spawn(async move{match request::<()>("DELETE",&format!("/api/runtime/workers/{id}"),None::<&()>).await{Ok(())=>{revoke.set(None);workers.restart();},Err(e)=>error.set(Some(e))}busy.set(false);});},"确认撤销"}
            }
        }
    }
}
