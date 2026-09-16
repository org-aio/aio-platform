use super::model::{Content, Entry, WriteEntry};
use crate::generated::worker::{model::Worker, view::request};
use az_ui_components::{
    button::{Button, ButtonVariant},
    dialog::{Dialog, DialogTitle},
    input::TextInput,
    select::{Select, SelectItem},
    textarea::Textarea,
};
use dioxus::prelude::*;

#[component]
pub(super) fn EntryForm(
    kind: String,
    initial: Option<Content>,
    workers: Vec<Worker>,
    on_close: EventHandler<()>,
    on_saved: EventHandler<()>,
) -> Element {
    let mut target = use_signal(|| {
        initial
            .as_ref()
            .map(|v| v.entry.target.clone())
            .unwrap_or_default()
    });
    let mut layer = use_signal(|| {
        initial
            .as_ref()
            .map(|v| v.entry.layer.clone())
            .unwrap_or("shared".into())
    });
    let mut content = use_signal(|| {
        initial
            .as_ref()
            .map(|v| {
                if v.entry.kind == "command" {
                    serde_json::from_str::<serde_json::Value>(&v.content)
                        .ok()
                        .and_then(|v| v["darwin"].as_str().map(str::to_owned))
                        .unwrap_or_default()
                } else {
                    v.content.clone()
                }
            })
            .unwrap_or_default()
    });
    let mut busy = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let mut layers = vec![
        SelectItem::new("shared", "所有设备共享"),
        SelectItem::new("os:darwin", "仅 macOS"),
        SelectItem::new("os:linux", "仅 Linux"),
    ];
    layers.extend(
        workers
            .iter()
            .filter(|w| w.status != "revoked")
            .map(|w| SelectItem::new(format!("device:{}", w.id), format!("仅 {}", w.label))),
    );
    let target_label = match kind.as_str() {
        "file" => "相对用户目录的文件路径",
        "command" => "启动命令",
        "env" | "paths" => "环境变量名（PATH 表示附加目录）",
        _ => "名称",
    };
    rsx! {
        Dialog{open:true,on_open_change:move|open:bool|if !open{on_close.call(())},
            div{class:"grid gap-3",
                div{class:"flex items-center justify-between",DialogTitle{if initial.is_some(){"编辑个人配置"}else{"新增个人配置"}}Button{variant:ButtonVariant::Ghost,onclick:move |_|on_close.call(()),"关闭"}}
                if initial.is_some(){p{class:"break-all","{target_label}：" {target()}}}else{TextInput{label:target_label,value:target(),on_change:move|v|target.set(v)}}
                Select{aria_label:"同步范围",value:layer(),options:layers,disabled:initial.is_some(),on_value_change:move|v|layer.set(v)}
                if kind=="command"{TextInput{label:"macOS 应用标识",placeholder:"例如 com.bot.pc.doubao",value:content(),on_change:move|v|content.set(v)}}else{
                    label{r#for:"personal-content","配置内容"}
                    Textarea{id:"personal-content",aria_label:"配置内容",rows:"12",value:content(),oninput:move|e:FormEvent|content.set(e.value())}
                    if kind=="env"||kind=="paths"{p{class:"text-sm","变量值按原文保存；PATH 填写目录的 JSON 数组。新终端加载后生效。"}}
                    if kind=="file"{p{class:"text-sm","用户主目录可写成 {{aio.home}}；保存后同步至已启用的设备，冲突会保留待处理。"}}
                }
                if let Some(message)=error(){p{role:"alert","{message}"}}
                Button{disabled:busy(),onclick:move |_|{
                    let initial=initial.clone();let kind=kind.clone();let target=target().trim().to_owned();let layer=layer();let content=content();busy.set(true);
                    spawn(async move{
                        let result=async{
                            let actual_kind=if kind=="env"&&target=="PATH"{"paths"}else{&kind}.to_owned();
                            let body=if actual_kind=="command"{serde_json::json!({"darwin":content}).to_string()}else{content};
                            let id=match &initial{Some(value)=>value.entry.id.clone(),None=>document::eval("return crypto.randomUUID();").await.map_err(|e|e.to_string())?.as_str().ok_or("无法生成 ID")?.to_owned()};
                            let format=if actual_kind=="file"&&(target.ends_with(".json")||target.ends_with(".jsonc")){"jsonc"}else{"text"};
                            let write=WriteEntry{id,expected:initial.as_ref().map(|v|v.entry.revision),kind:actual_kind,target,layer,format:format.into(),secret:initial.as_ref().is_none_or(|v|v.entry.secret),executable:initial.as_ref().is_some_and(|v|v.entry.executable),deleted:false,content:body};
                            request::<Entry>("POST","/api/runtime/personal-config/entries",Some(&write)).await
                        }.await;
                        match result{Ok(_)=>on_saved.call(()),Err(e)=>error.set(Some(e))}busy.set(false);
                    });
                },"保存并同步"}
            }
        }
    }
}
