use super::model::Content;
use crate::generated::worker::{
    model::{SubmitTask, Task, Worker},
    view::request,
};
use az_ui_components::{
    button::{Button, ButtonVariant},
    dialog::{Dialog, DialogTitle},
    input::TextInput,
    select::{Select, SelectItem},
};
use dioxus::prelude::*;
#[component]
pub(super) fn AssetRestore(
    asset: Content,
    workers: Vec<Worker>,
    on_close: EventHandler<()>,
) -> Element {
    let options = workers
        .iter()
        .filter(|w| {
            w.status != "revoked" && w.capabilities.iter().any(|c| c == "space.archive-restore")
        })
        .map(|w| SelectItem::new(w.id.clone(), w.label.clone()))
        .collect::<Vec<_>>();
    let mut device = use_signal(|| options.first().map(|v| v.value.clone()).unwrap_or_default());
    let mut path = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let mut done = use_signal(|| false);
    let metadata = serde_json::from_str::<serde_json::Value>(&asset.content).unwrap_or_default();
    let snapshot = metadata["snapshotId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    rsx! {Dialog{open:true,on_open_change:move|open:bool|if !open{on_close.call(())},div{class:"grid gap-3",
        div{class:"flex items-center justify-between",DialogTitle{"恢复个人资源"}Button{variant:ButtonVariant::Ghost,onclick:move |_|on_close.call(()),"关闭"}}
        p{class:"break-all","{asset.entry.target}"}p{"恢复目录必须不存在，内容保留原始路径层级。安装包不会自动执行。"}
        Select{aria_label:"恢复到设备",value:device(),options,on_value_change:move|v|device.set(v)}
        TextInput{label:"目标设备上的新目录",value:path(),on_change:move|v|path.set(v)}
        if let Some(message)=error(){p{role:"alert","{message}"}}
        if done(){p{role:"status","恢复任务已提交，可在“我的设备”查看结果。"}}
        Button{disabled:busy()||done()||device().is_empty()||path().trim().is_empty(),onclick:move |_|{let worker=device();let path=path();let snapshot=snapshot.clone();busy.set(true);spawn(async move{let result=async{let id=document::eval("return crypto.randomUUID();").await.map_err(|e|e.to_string())?.as_str().ok_or("无法生成 ID")?.to_owned();request::<Task>("POST","/api/runtime/workers/tasks",Some(&SubmitTask{id,worker_id:worker,capability:"space.archive-restore".into(),input:serde_json::json!({"snapshot":snapshot,"path":path})})).await}.await;match result{Ok(_)=>done.set(true),Err(e)=>error.set(Some(e))}busy.set(false);});},"恢复到此设备"}
    }}}
}
