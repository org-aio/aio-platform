use az_ui_components::collection_tree::{
    CollectionTree, CollectionTreeData, CollectionTreeItemContext, CollectionTreeNode,
};
use dioxus::prelude::*;

use crate::{
    CompiledTree, ProgramImage, RuntimeRecordPage,
    page_runtime::{record_field, value_to_text},
};

const ROOT_KEY: &str = "all";

#[derive(Clone, Debug, PartialEq)]
struct RuntimeTreeItem {
    key: String,
    label: String,
    record_id: Option<String>,
    parent_key: Option<String>,
}

#[component]
pub(crate) fn RuntimeTree(
    tree: CompiledTree,
    image: ProgramImage,
    page: RuntimeRecordPage,
    mut selected_record: Signal<Option<String>>,
) -> Element {
    let data = match runtime_tree_data(&tree, &image, &page) {
        Ok(data) => data,
        Err(error) => {
            return rsx! {
                div { class: "aio-runtime-table-state is-error", role: "alert", "{error}" }
            };
        }
    };

    rsx! {
        CollectionTree::<RuntimeTreeItem> {
            class: "aio-runtime-tree__items",
            aria_label: "分类树",
            data,
            selected_key: Some(selected_tree_key(selected_record())),
            item_key: |item: RuntimeTreeItem| item.key,
            on_select: move |item: RuntimeTreeItem| {
                selected_record.set(item.record_id);
            },
            render_item: |item: CollectionTreeItemContext<RuntimeTreeItem>| {
                rsx! { span { class: "aio-runtime-tree__label", "{item.item.label}" } }
            }
        }
    }
}

fn runtime_tree_data(
    tree: &CompiledTree,
    image: &ProgramImage,
    page: &RuntimeRecordPage,
) -> Result<CollectionTreeData<RuntimeTreeItem>, String> {
    let Some(model) = image.models.get(&tree.model_id) else {
        return Err("找不到分类模型".to_owned());
    };
    let items = page
        .d
        .iter()
        .map(|record| RuntimeTreeItem {
            key: record_key(&record.id),
            label: record_field(record, model, tree.label_field_id)
                .map(value_to_text)
                .unwrap_or_else(|| "未命名".to_owned()),
            record_id: Some(record.id.clone()),
            parent_key: tree.parent_field_id.and_then(|parent_field_id| {
                record_field(record, model, parent_field_id)
                    .map(value_to_text)
                    .filter(|parent_id| !parent_id.trim().is_empty())
                    .map(|parent_id| record_key(&parent_id))
            }),
        })
        .collect::<Vec<_>>();
    let data = CollectionTreeData::from_parented_collection(
        items,
        |item| item.key.clone(),
        |item| item.parent_key.clone(),
    )?;
    let CollectionTreeData::Tree(children) = data else {
        return Err("分类集合未转换为树".to_owned());
    };
    let root = RuntimeTreeItem {
        key: ROOT_KEY.to_owned(),
        label: "全部".to_owned(),
        record_id: None,
        parent_key: None,
    };
    Ok(CollectionTreeData::Tree(vec![CollectionTreeNode::branch(
        root, children,
    )]))
}

fn selected_tree_key(record_id: Option<String>) -> String {
    record_id.map_or_else(|| ROOT_KEY.to_owned(), |record_id| record_key(&record_id))
}

fn record_key(record_id: &str) -> String {
    format!("record:{record_id}")
}
