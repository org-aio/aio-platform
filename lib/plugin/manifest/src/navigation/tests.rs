use serde_json::{Value, json};

use super::parse_page_definitions;

fn leaf(id: &str) -> Value {
    json!({"id":id,"label":id,"required_permission":"reports.read",
        "body":{"kind":"frontend","entry":"index.html"}})
}

fn scene(children: Value) -> Value {
    json!({"id":"business","label":"业务","children":children})
}

fn parse(value: Value) -> anyhow::Result<Vec<crate::PageDefinition>> {
    parse_page_definitions(&serde_json::to_vec(&value)?)
}

#[test]
fn expands_shared_branches_in_order_and_preserves_permissions() -> anyhow::Result<()> {
    let pages = parse(scene(json!([
        {"id":"operations","label":"运营","icon":"folder","children":[
            {"id":"analysis","label":"分析","children":[leaf("dashboard"),leaf("reports")]}
        ]}, leaf("home")
    ])))?;
    assert_eq!(
        pages.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        ["dashboard", "reports", "home"]
    );
    assert_eq!(pages[0].scene.id, "business");
    assert_eq!(pages[0].menu_path, pages[1].menu_path);
    assert_eq!(
        pages[0]
            .menu_path
            .iter()
            .map(|g| g.id.as_str())
            .collect::<Vec<_>>(),
        ["operations", "analysis"]
    );
    assert_eq!(pages[0].menu_path[0].icon.as_deref(), Some("folder"));
    assert_eq!(
        pages[0].required_permission.as_deref(),
        Some("reports.read")
    );
    assert!(pages[2].menu_path.is_empty());
    // 展开后的协议可再次经过相同入口，持久化语义不变。
    assert_eq!(parse(serde_json::to_value(&pages)?)?, pages);
    Ok(())
}

#[test]
fn supports_multiple_scenes_and_cross_plugin_group_merging() -> anyhow::Result<()> {
    let contribution = |id| scene(json!([{"id":"analysis","label":"分析","children":[leaf(id)]}]));
    let mut pages = parse(contribution("dashboard"))?;
    pages.extend(parse(contribution("reports"))?);
    crate::validate_page_definitions(&pages)?;
    let multiple = parse(json!([scene(json!([leaf("home")])),
        {"id":"workspace","label":"工作区","children":[leaf("tasks")]}]))?;
    assert_eq!(multiple[1].scene.id, "workspace");
    pages[1].menu_path[0].label = "不同标题".into();
    assert!(crate::validate_page_definitions(&pages).is_err());
    Ok(())
}

#[test]
fn rejects_ambiguous_nodes_unknown_fields_and_empty_branches() {
    let mut both = leaf("both");
    both["children"] = json!([leaf("child")]);
    let mut typo = leaf("typo");
    typo["chidlren"] = json!([]);
    let mut override_scene = leaf("override");
    override_scene["scene"] = json!({"id":"system","label":"系统"});
    for invalid in [
        scene(json!([])),
        scene(json!([{"id":"empty","label":"空目录","children":[]}])),
        scene(json!([{"id":"missing","label":"无页面体"}])),
        scene(
            json!([{"id":"group","label":"目录","required_permission":"admin","children":[leaf("page")]}]),
        ),
        scene(json!([both])),
        scene(json!([typo])),
        scene(json!([override_scene])),
    ] {
        assert!(parse(invalid.clone()).is_err(), "应拒绝 {invalid}");
    }
}

#[test]
fn rejects_cycles_conflicts_duplicate_pages_and_invalid_entries() {
    let mut bad_entry = leaf("bad-entry");
    bad_entry["body"]["entry"] = json!("../secret.html");
    for invalid in [
        scene(json!([leaf("same"), leaf("same")])),
        scene(json!([{"id":"group","label":"目录","children":[
            {"id":"group","label":"目录","children":[leaf("page")]}
        ]}])),
        scene(json!([{"id":"page","label":"目录","children":[leaf("page")]}])),
        json!([scene(json!([leaf("one")])),{"id":"business","label":"冲突","children":[leaf("two")]}]),
        scene(json!([bad_entry])),
    ] {
        assert!(parse(invalid.clone()).is_err(), "应拒绝 {invalid}");
    }
}

#[test]
fn limits_tree_depth() -> anyhow::Result<()> {
    let mut node = leaf("page");
    for index in 0..8 {
        node = json!({"id":format!("group-{index}"),"label":"目录","children":[node]});
    }
    assert_eq!(parse(scene(json!([node.clone()])))?[0].menu_path.len(), 8);
    node = json!({"id":"too-deep","label":"目录","children":[node]});
    assert!(
        parse(scene(json!([node])))
            .unwrap_err()
            .to_string()
            .contains("超过 8 层")
    );
    Ok(())
}

#[test]
fn generated_kmp_template_uses_a_valid_tree() -> anyhow::Result<()> {
    let bytes = include_bytes!(
        "../../../../../cli/templates/plugin/fullstack/kotlin/backend/service/resources/pages.json"
    );
    let document: Value = serde_json::from_slice(bytes)?;
    assert!(document["children"].is_array());
    let pages = parse_page_definitions(bytes)?;
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].id, "kmp-fullstack");
    Ok(())
}
