use super::model::{Catalog, Content, Entry, WriteEntry};
use crate::generated::worker::{model::Worker, view::request};
use az_ui_components::{
    button::{Button, ButtonVariant},
    dialog::{Dialog, DialogTitle},
    input::TextInput,
};
use dioxus::prelude::*;
#[component]
pub(crate) fn PersonalConfigPanel(on_close: EventHandler<()>) -> Element {
    let mut tab = use_signal(|| "file".to_owned());
    let mut search = use_signal(String::new);
    let mut page = use_signal(|| 0usize);
    let mut catalog = use_resource(move || async {
        request::<Catalog>("GET", "/api/runtime/personal-config/catalog", None::<&()>).await
    });
    let mut workers = use_resource(move || async {
        request::<Vec<Worker>>("GET", "/api/runtime/workers", None::<&()>).await
    });
    let mut editor = use_signal(|| None::<(String, Option<Content>)>);
    let mut history = use_signal(|| None::<Entry>);
    let mut deleting = use_signal(|| None::<Entry>);
    let mut asset = use_signal(|| None::<Content>);
    let mut error = use_signal(|| None::<String>);
    let mut notice = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);
    use_future(move || async move {
        loop {
            let _ = document::eval(
                "await new Promise(resolve => setTimeout(resolve, 5000)); return true;",
            )
            .await;
            catalog.restart();
            workers.restart();
        }
    });
    let devices = workers
        .read()
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .cloned()
        .unwrap_or_default();
    let saved = move |()| {
        editor.set(None);
        history.set(None);
        catalog.restart();
        notice.set(Some("已保存，在线设备将自动同步。".into()));
    };
    rsx! {
        Dialog{open:true,on_open_change:move|open:bool|if !open{on_close.call(())},class:"w-full max-w-4xl",
            div{class:"grid gap-3",
                div{class:"flex items-center justify-between gap-2",DialogTitle{"个人配置"}Button{variant:ButtonVariant::Ghost,onclick:move |_|on_close.call(()),"关闭"}}
                p{class:"text-sm","在自己的设备间同步配置。共享、系统和单设备设置逐层覆盖；数据只属于当前租户下的当前账号。"}
                div{class:"flex flex-wrap gap-2",role:"tablist",aria_label:"个人配置分类",for (value,label) in [("file","配置文件"),("env","环境变量"),("function","Bash 函数"),("command","应用命令"),("asset","个人资源"),("devices","同步设备")]{Button{variant:if tab()==value{ButtonVariant::Primary}else{ButtonVariant::Outline},role:"tab",aria_selected:tab()==value,onclick:move |_|{tab.set(value.into());page.set(0);search.set(String::new());},"{label}"}}}
                if let Some(message)=error(){p{role:"alert","{message}"}}
                if let Some(message)=notice(){p{role:"status","{message}"}}
                if tab()!="devices"{
                    div{class:"flex flex-wrap items-end gap-2",TextInput{label:"搜索配置",value:search(),on_change:move|v|{search.set(v);page.set(0);}}
                        if tab()!="asset"{Button{onclick:move |_|editor.set(Some((tab(),None))),"新增"}}
                        Button{variant:ButtonVariant::Outline,onclick:move |_|catalog.restart(),"刷新"}
                    }
                    if tab()=="file"{details{summary{"从本机导入"}p{"首次在设备运行 aio-space config-enable，再运行 aio-space config-import --preview 查看 chezmoi 候选，去掉 --preview 后导入。单个文件用 aio-space config-add --path 路径。"}}}
                    if tab()=="asset"{p{class:"text-sm","在已配对设备运行 aio-space asset-add --path 文件或目录，即可归档至 AIO 并在此管理。原件保留。"}}
                }
                match catalog.read().as_ref(){
                    Some(Ok(value))=>{
                        if tab()=="devices"{rsx!{super::devices_view::DevicesView{workers:devices.clone(),reports:value.devices.clone(),on_change:move |_|{catalog.restart();workers.restart();}}}}
                        else{
                            let items=value.entries.iter().filter(|e|(e.kind==tab()||(tab()=="env"&&e.kind=="paths"))&&e.target.to_lowercase().contains(&search().to_lowercase())).collect::<Vec<_>>();
                            let count=items.len();let offset=(page()*30).min(count.saturating_sub(1)/30*30);
                            rsx!{
                                if count==0{p{"暂无配置。添加后会同步到已启用的设备。"}}
                                div{class:"grid gap-2",for entry in items.into_iter().skip(offset).take(30){
                                    section{key:"{entry.id}",class:"grid gap-2 border-b pb-3",
                                        strong{class:"break-all","{entry.target}"}
                                        small{class:"break-all",{scope_label(&entry.layer,&devices)}" · 版本 {entry.revision}" if entry.deleted{" · 已删除"}}
                                        div{class:"flex flex-wrap gap-2",
                                            if entry.format.starts_with("yjs-"){small{"Space 文件同步"}}
                                            if !entry.deleted && !entry.format.starts_with("yjs-"){Button{variant:ButtonVariant::Outline,disabled:busy(),onclick:{let entry=entry.clone();move |_|{let entry=entry.clone();busy.set(true);spawn(async move{match request::<Content>("GET",&format!("/api/runtime/personal-config/entries/{}",entry.id),None::<&()>).await{Ok(value)=>if entry.kind=="asset"{asset.set(Some(value))}else{editor.set(Some((entry.kind,Some(value))))},Err(e)=>error.set(Some(e))}busy.set(false);});}},if entry.kind=="asset"{"恢复到设备"}else{"编辑"}}}
                                            if !entry.format.starts_with("yjs-"){Button{variant:ButtonVariant::Ghost,disabled:busy()||!catalog.finished(),onclick:{let entry=entry.clone();move |_|history.set(Some(entry.clone()))},"历史版本"}}
                                            if !entry.deleted{Button{variant:ButtonVariant::Ghost,disabled:busy()||!catalog.finished(),onclick:{let entry=entry.clone();move |_|deleting.set(Some(entry.clone()))},"删除"}}
                                        }
                                    }
                                }}
                                if count>30{div{class:"flex items-center justify-between gap-2",Button{variant:ButtonVariant::Outline,disabled:offset==0,onclick:move |_|page.set(page().saturating_sub(1)),"上一页"}small{"{count} 项 · 第 {offset / 30 + 1} 页"}Button{variant:ButtonVariant::Outline,disabled:offset+30>=count,onclick:move |_|page.set(page()+1),"下一页"}}}
                            }
                        }
                    },
                    Some(Err(e))=>rsx!{p{role:"alert","{e}"}},None=>rsx!{p{"正在加载个人配置…"}}
                }
            }
        }
        if let Some((kind,initial))=editor(){super::entry_form::EntryForm{kind,initial,workers:devices.clone(),on_close:move |_|editor.set(None),on_saved:saved}}
        if let Some(entry)=history(){super::history_view::HistoryView{entry,on_close:move |_|history.set(None),on_saved:saved}}
        if let Some(value)=asset(){super::assets_view::AssetRestore{asset:value,workers:devices,on_close:move |_|asset.set(None)}}
        if let Some(entry)=deleting(){Dialog{open:true,on_open_change:move|open:bool|if !open{deleting.set(None)},DialogTitle{"删除配置"}p{class:"break-all","确认删除 {entry.target}？同步设备会移除此项，已有文件先备份；历史版本仍可恢复。资源仅移除目录记录，不删除归档。"}if let Some(message)=error(){p{role:"alert","{message}"}}Button{disabled:busy(),onclick:move |_|{let entry=entry.clone();busy.set(true);spawn(async move{let write=WriteEntry{id:entry.id,expected:Some(entry.revision),kind:entry.kind,target:entry.target,layer:entry.layer,format:entry.format,secret:entry.secret,executable:entry.executable,deleted:true,content:String::new()};match request::<Entry>("POST","/api/runtime/personal-config/entries",Some(&write)).await{Ok(_)=>{deleting.set(None);catalog.restart();notice.set(Some("已删除，在线设备将自动同步。".into()));},Err(e)=>error.set(Some(e))}busy.set(false);});},"确认删除"}}}
    }
}
fn scope_label(layer: &str, workers: &[Worker]) -> String {
    match layer {
        "shared" => "所有设备".into(),
        "os:darwin" => "macOS".into(),
        "os:linux" => "Linux".into(),
        _ => workers
            .iter()
            .find(|w| Some(w.id.as_str()) == layer.strip_prefix("device:"))
            .map(|w| w.label.clone())
            .unwrap_or_else(|| "指定设备".into()),
    }
}
