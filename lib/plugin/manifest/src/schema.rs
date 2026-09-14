use schemars::{Schema, schema_for};
use serde_json::Value;

use crate::{ComponentResponse, PageActionResult, PageDocument, PluginRequest, RepositoryManifest};

pub struct PluginSchema {
    pub file_name: &'static str,
    pub schema: Schema,
}

pub fn schemas() -> Vec<PluginSchema> {
    vec![
        document(
            "repository-manifest.schema.json",
            schema_for!(RepositoryManifest),
        ),
        document("page-definitions.schema.json", schema_for!(PageDocument)),
        document("plugin-request.schema.json", schema_for!(PluginRequest)),
        document(
            "component-response.schema.json",
            schema_for!(ComponentResponse),
        ),
        document(
            "page-action-result.schema.json",
            schema_for!(PageActionResult),
        ),
    ]
}

fn document(file_name: &'static str, mut schema: Schema) -> PluginSchema {
    let object = schema.ensure_object();
    object.insert(
        "$id".to_owned(),
        Value::String(format!(
            "https://raw.githubusercontent.com/zjarlin/aio-platform/main/docs/plugin/schema/{file_name}"
        )),
    );
    object.insert(
        "$comment".to_owned(),
        Value::String("AIO Plugin JSON contract aio:plugin@1.0.0".to_owned()),
    );
    PluginSchema { file_name, schema }
}
