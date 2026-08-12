use super::*;

pub(super) fn toggle_model_audit_field(
    model: ModelDefinition,
    kind: crate::AuditFieldKind,
    enabled: bool,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
) {
    let patches = match crate::model_audit::model_audit_patches(&model, kind, enabled) {
        Ok(patches) => patches,
        Err(error) => {
            status.set(Some(error));
            return;
        }
    };
    if patches.is_empty() {
        return;
    }
    submit_patches(
        api_base_url,
        program_id,
        version,
        patches,
        generation,
        status,
    );
}
