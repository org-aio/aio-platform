use std::collections::BTreeMap;

use crate::{CompiledRoute, ImageTarget, PROGRAM_SCHEMA_VERSION, ProgramImage, SymbolId};

use super::*;

fn empty_image(revision_id: &str) -> ProgramImage {
    ProgramImage {
        schema_version: PROGRAM_SCHEMA_VERSION,
        compiler_version: PROGRAM_COMPILER_VERSION.to_owned(),
        content_hash: format!("hash-{revision_id}"),
        program_id: SymbolId::new(),
        name: "records".to_owned(),
        title: "记录".to_owned(),
        revision_id: revision_id.to_owned(),
        target: ImageTarget::Universal,
        menus: Vec::new(),
        permissions: Vec::new(),
        pages: BTreeMap::new(),
        client_functions: BTreeMap::new(),
        server_functions: BTreeMap::new(),
        models: BTreeMap::new(),
        routes: Vec::new(),
        dependencies: BTreeMap::new(),
    }
}

#[test]
fn prebuilt_router_resolves_without_database() -> Result<()> {
    let route_id = SymbolId::new();
    let page_id = SymbolId::new();
    let mut image = empty_image("revision");
    image.routes = vec![CompiledRoute {
        id: route_id,
        name: "record".to_owned(),
        path: "/records/{id}".to_owned(),
        page_id,
        required_permissions: Vec::new(),
    }];
    let runtime = RuntimeProgramImage::build(image)?;
    let matched = runtime.route("/records/42")?;
    assert_eq!(matched.route_id, route_id);
    assert_eq!(matched.parameters, vec![("id".to_owned(), "42".to_owned())]);
    Ok(())
}

#[test]
fn in_flight_request_keeps_old_arc_after_atomic_swap() -> Result<()> {
    let old = Arc::new(RuntimeProgramImage::build(empty_image("revision-old"))?);
    let slot = ArcSwapOption::new(Some(Arc::clone(&old)));
    let in_flight = slot.load_full().context("请求应取得活动 Image")?;
    let new = Arc::new(RuntimeProgramImage::build(empty_image("revision-new"))?);
    slot.store(Some(new));

    assert_eq!(in_flight.image().revision_id, "revision-old");
    let active = slot.load_full().context("发布后应存在新 Image")?;
    assert_eq!(active.image().revision_id, "revision-new");
    Ok(())
}

#[test]
fn postgres_listener_retry_delay_is_bounded() {
    assert_eq!(postgres_listener_retry_delay(1), Duration::from_secs(1));
    assert_eq!(postgres_listener_retry_delay(2), Duration::from_secs(2));
    assert_eq!(postgres_listener_retry_delay(6), Duration::from_secs(32));
    assert_eq!(postgres_listener_retry_delay(20), Duration::from_secs(32));
}
