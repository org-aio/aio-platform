use super::{
    model::{SubmitTask, Task, Worker},
    view::request,
};
use az_ui_components::{
    button::Button,
    dialog::{Dialog, DialogTitle},
    input::TextInput,
};
use dioxus::prelude::*;

#[component]
pub(super) fn TaskForm(
    worker: Worker,
    capability: String,
    on_close: EventHandler<()>,
    on_submitted: EventHandler<()>,
) -> Element {
    let mut path = use_signal(String::new);
    let mut snapshot = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let restore = capability == "space.archive-restore";
    let list = capability == "space.archive-list";
    let cleanup = capability == "space.clean";
    rsx! {
        Dialog{open:true,on_open_change:move|open:bool|if !open{on_close.call(())},
            div{class:"grid gap-3",DialogTitle{"在 {worker.label} 上执行任务"}
                if cleanup{p{"确认删除指定项目的 target 构建缓存。源码保留，下次构建会重新生成。"}}
                if !list{TextInput{label:if restore{"恢复到新目录"}else{"设备上的路径（留空扫描用户目录）"},value:path(),on_change:move |v|path.set(v)}}
                if restore{TextInput{label:"快照 ID",value:snapshot(),on_change:move |v|snapshot.set(v)}}
                if let Some(message)=error(){p{role:"alert","{message}"}}
                Button{disabled:busy(),onclick:move |_|{
                    let worker=worker.id.clone();let capability=capability.clone();let path=path();let snapshot=snapshot();busy.set(true);
                    spawn(async move{
                        let result=async{
                            let id=document::eval("return crypto.randomUUID();").await.map_err(|e|e.to_string())?;
                            let id=id.as_str().ok_or("无法生成任务 ID")?.to_owned();
                            let input=serde_json::json!({"path":path,"snapshot":snapshot,"depth":1});
                            request::<Task>("POST","/api/runtime/workers/tasks",Some(&SubmitTask{id,worker_id:worker,capability,input})).await
                        }.await;
                        match result{Ok(_)=>on_submitted.call(()),Err(e)=>error.set(Some(e))}busy.set(false);
                    });
                },if cleanup{"确认清理"}else{"提交任务"}}
            }
        }
    }
}
