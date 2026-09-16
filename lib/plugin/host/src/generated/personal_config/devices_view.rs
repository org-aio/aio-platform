use super::model::{Access, Resolution, SyncDevice};
use crate::generated::worker::{model::Worker, view::request};
use az_ui_components::{
    button::{Button, ButtonVariant},
    dialog::{Dialog, DialogTitle},
};
use dioxus::prelude::*;
#[component]
pub(super) fn DevicesView(
    workers: Vec<Worker>,
    reports: Vec<SyncDevice>,
    on_change: EventHandler<()>,
) -> Element {
    let mut error = use_signal(|| None::<String>);
    let mut disabling = use_signal(|| None::<Worker>);
    let mut resolving = use_signal(|| None::<Resolution>);
    let mut busy = use_signal(|| false);
    rsx! {div{class:"grid gap-3",
        p{"每台设备单独配对，沿用当前账号。在设备运行 aio-space config-enable 后开始同步。"}
        if workers.is_empty(){p{"还没有配对设备，请先在客户端运行 aio-space connect。"}}
        for worker in workers {
            section{key:"{worker.id}",class:"grid gap-2 border-b pb-3",
                strong{"{worker.label} · {worker.status}"}
                if let Some(report)=reports.iter().find(|r|r.id==worker.id){
                    p{"同步状态：" {match report.report["phase"].as_str(){Some("complete")=>"已同步",Some("conflict")=>"有冲突",Some("failed")=>"需要处理",_=>"等待首次同步"}}}
                    if let Some(conflicts)=report.report["conflicts"].as_array(){for conflict in conflicts{
                        div{class:"grid gap-2",p{class:"break-all","冲突：" {conflict["target"].as_str().unwrap_or_default()}}
                            div{class:"flex flex-wrap gap-2",for (side,label) in [("local","保留此设备内容"),("remote","采用云端内容")]{Button{variant:ButtonVariant::Outline,onclick:{let choice=Resolution{device:worker.id.clone(),entry:conflict["id"].as_str().unwrap_or_default().into(),local:conflict["local"].as_str().map(str::to_owned),remote:conflict["remote"].as_str().unwrap_or_default().into(),side:side.into()};move |_|resolving.set(Some(choice.clone()))},"{label}"}}}
                        }
                    }}
                    if let Some(errors)=report.report["errors"].as_array(){for error in errors{p{role:"alert",class:"break-all",{error["target"].as_str().unwrap_or_default()}"："{error["reason"].as_str().unwrap_or_default()}}}}
                }else{p{"等待首次同步"}}
                if worker.capabilities.iter().any(|c|c=="config.sync")&&worker.status!="revoked"{Button{variant:ButtonVariant::Outline,onclick:move |_|disabling.set(Some(worker.clone())),"停用此设备同步"}}
            }
        }
        if let Some(message)=error(){p{role:"alert","{message}"}}
    }
    if let Some(worker)=disabling(){Dialog{open:true,on_open_change:move|open:bool|if !open{disabling.set(None)},DialogTitle{"停用配置同步"}p{"{worker.label} 将停止同步，本机现有配置保留。"}Button{disabled:busy(),onclick:move |_|{let id=worker.id.clone();busy.set(true);spawn(async move{match request::<()>("PUT",&format!("/api/runtime/personal-config/devices/{id}"),Some(&Access{enabled:false})).await{Ok(())=>{disabling.set(None);on_change.call(());},Err(e)=>error.set(Some(e))}busy.set(false);});},"确认停用"}}}
    if let Some(choice)=resolving(){Dialog{open:true,on_open_change:move|open:bool|if !open{resolving.set(None)},DialogTitle{"处理同步冲突"}p{"确认后将应用所选版本，原文件会先备份。如果两端又有修改，此选择自动失效。"}Button{disabled:busy(),onclick:move |_|{let choice=choice.clone();busy.set(true);spawn(async move{match request::<()>("POST","/api/runtime/personal-config/resolve",Some(&choice)).await{Ok(())=>{resolving.set(None);on_change.call(());},Err(e)=>error.set(Some(e))}busy.set(false);});},"确认使用所选版本"}}}
    }
}
