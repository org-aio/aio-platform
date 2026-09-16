use super::model::{Content, Entry, WriteEntry};
use crate::generated::worker::view::request;
use az_ui_components::{
    button::{Button, ButtonVariant},
    dialog::{Dialog, DialogTitle},
};
use dioxus::prelude::*;
#[component]
pub(super) fn HistoryView(
    entry: Entry,
    on_close: EventHandler<()>,
    on_saved: EventHandler<()>,
) -> Element {
    let query = entry.id.clone();
    let history = use_resource(move || {
        let id = query.clone();
        async move {
            request::<Vec<Entry>>(
                "GET",
                &format!("/api/runtime/personal-config/entries/{id}/history"),
                None::<&()>,
            )
            .await
        }
    });
    let mut selected = use_signal(|| None::<Content>);
    let mut error = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);
    rsx! {Dialog{open:true,on_open_change:move|open:bool|if !open{on_close.call(())},div{class:"grid gap-3",
        div{class:"flex items-center justify-between",DialogTitle{"历史版本"}Button{variant:ButtonVariant::Ghost,onclick:move |_|on_close.call(()),"关闭"}}
        p{class:"break-all","{entry.target} · 当前版本 {entry.revision}"}
        match history.read().as_ref(){
            Some(Ok(items))=>rsx!{if items.is_empty(){p{"还没有历史版本。"}}div{class:"flex flex-wrap gap-2",for version in items{Button{variant:ButtonVariant::Outline,disabled:busy(),onclick:{let id=entry.id.clone();let revision=version.revision;move |_|{let id=id.clone();busy.set(true);spawn(async move{match request::<Content>("GET",&format!("/api/runtime/personal-config/entries/{id}?revision={revision}"),None::<&()>).await{Ok(value)=>selected.set(Some(value)),Err(e)=>error.set(Some(e))}busy.set(false);});}},"版本 {version.revision}"}}}},
            Some(Err(e))=>rsx!{p{role:"alert","{e}"}},None=>rsx!{p{"正在读取…"}}
        }
        if let Some(value)=selected(){
            pre{class:"overflow-auto text-xs", "{value.content}"}
            p{"确认后会产生一个新版本；已启用同步的设备将收到更新。"}
            Button{disabled:busy(),onclick:move |_|{let entry=entry.clone();let value=value.clone();busy.set(true);spawn(async move{let write=WriteEntry{id:entry.id,expected:Some(entry.revision),kind:entry.kind,target:entry.target,layer:entry.layer,format:value.entry.format,secret:value.entry.secret,executable:value.entry.executable,deleted:value.entry.deleted,content:value.content};match request::<Entry>("POST","/api/runtime/personal-config/entries",Some(&write)).await{Ok(_)=>on_saved.call(()),Err(e)=>error.set(Some(e))}busy.set(false);});},"确认恢复此版本"}
        }
        if let Some(message)=error(){p{role:"alert","{message}"}}
    }}}
}
