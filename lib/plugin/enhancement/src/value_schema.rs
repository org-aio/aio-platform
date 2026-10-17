use serde_json::Value;

pub(crate) fn same(left: &Option<schemars::Schema>, right: &Option<schemars::Schema>) -> bool {
    let any = Value::Bool(true);
    left.as_ref()
        .map(schemars::Schema::as_value)
        .unwrap_or(&any)
        == right
            .as_ref()
            .map(schemars::Schema::as_value)
            .unwrap_or(&any)
}
