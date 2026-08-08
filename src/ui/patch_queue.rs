use super::*;

pub(super) struct PendingStudioPatch {
    api_base_url: String,
    program_id: String,
    base_version: i64,
    patches: Vec<GraphPatch>,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
}

thread_local! {
    static STUDIO_PATCH_QUEUE: RefCell<VecDeque<PendingStudioPatch>> = RefCell::new(VecDeque::new());
    static STUDIO_PATCH_ACTIVE: Cell<bool> = const { Cell::new(false) };
    static STUDIO_PATCH_VERSIONS: RefCell<BTreeMap<String, i64>> = const { RefCell::new(BTreeMap::new()) };
}

pub(super) fn submit_patches(
    api_base_url: String,
    program_id: String,
    base_version: i64,
    patches: Vec<GraphPatch>,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
) {
    STUDIO_PATCH_QUEUE.with(|queue| {
        queue.borrow_mut().push_back(PendingStudioPatch {
            api_base_url,
            program_id,
            base_version,
            patches,
            generation,
            status,
        });
    });
    let should_start = STUDIO_PATCH_ACTIVE.with(|active| {
        if active.get() {
            false
        } else {
            active.set(true);
            true
        }
    });
    if should_start {
        // Dialog 提交后会立即卸载，保存队列必须挂在根作用域继续执行。
        dioxus::dioxus_core::spawn_forever(drain_studio_patch_queue());
    }
}

pub(super) async fn drain_studio_patch_queue() {
    loop {
        let pending = STUDIO_PATCH_QUEUE.with(|queue| queue.borrow_mut().pop_front());
        let Some(pending) = pending else {
            STUDIO_PATCH_ACTIVE.with(|active| active.set(false));
            return;
        };
        let PendingStudioPatch {
            api_base_url,
            program_id,
            base_version,
            patches,
            mut generation,
            mut status,
        } = pending;
        let base_version = STUDIO_PATCH_VERSIONS.with(|versions| {
            versions
                .borrow()
                .get(&program_id)
                .copied()
                .map_or(base_version, |current| current.max(base_version))
        });
        let path = "/api/studio/program/draft";
        let batch = GraphPatchBatch {
            base_version,
            patches,
            origin: PatchOrigin::Studio,
        };
        match patch_api::<_, DraftSnapshot>(&api_base_url, &path, &batch).await {
            Ok(draft) => {
                STUDIO_PATCH_VERSIONS.with(|versions| {
                    versions.borrow_mut().insert(program_id, draft.version);
                });
                generation.with_mut(|value| *value = value.saturating_add(1));
                status.set(Some("已保存，等待自动发布".to_owned()));
            }
            Err(error) => {
                STUDIO_PATCH_VERSIONS.with(|versions| {
                    versions.borrow_mut().remove(&program_id);
                });
                if error.starts_with("draft version conflict") {
                    STUDIO_PATCH_QUEUE.with(|queue| {
                        queue
                            .borrow_mut()
                            .retain(|item| item.program_id != program_id);
                    });
                    status.set(Some("草稿已被其他操作更新，已刷新，请重试".to_owned()));
                } else {
                    status.set(Some(error));
                }
                generation.with_mut(|value| *value = value.saturating_add(1));
            }
        }
    }
}

pub(super) fn empty_panel(message: &str) -> Element {
    rsx! { div { class: "grid min-h-48 place-items-center rounded-md border border-dashed p-6 text-sm text-muted-foreground", "{message}" } }
}

pub(super) fn form_text(event: &FormEvent, name: &str) -> String {
    match event.get_first(name) {
        Some(dioxus::html::FormValue::Text(value)) => value,
        _ => String::new(),
    }
}
