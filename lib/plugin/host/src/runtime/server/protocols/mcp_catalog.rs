use std::sync::Arc;

use az_plugin_enhancement::EnhancementContract;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rmcp::{
    ErrorData,
    model::{ListToolsResult, PaginatedRequestParams, Tool, ToolAnnotations},
};

use super::{
    super::{
        RuntimeState,
        capabilities::{CapabilityEffect, RuntimeCapability},
    },
    access,
    mcp_schema::envelope,
};
use crate::identity::SessionContext;

pub(super) enum Target {
    Capability(String),
    Enhancement(String),
}

// runtime 前缀区分两种权限域；增强路径以 v 加版本号起始，不会产生同名工具。
pub(super) fn target(name: &str) -> Result<Target, ErrorData> {
    if name.len() > 136 || name.contains('/') {
        return Err(ErrorData::invalid_params("能力工具名称无效", None));
    }
    let native = name.strip_prefix("runtime.");
    let path = format!("/{}", native.unwrap_or(name).replace('.', "/"));
    az_plugin_contract::validate_enhancement_path(&path)
        .map_err(|_| ErrorData::invalid_params("能力工具名称无效", None))?;
    Ok(if native.is_some() {
        Target::Capability(path)
    } else {
        Target::Enhancement(path)
    })
}

enum Description<'a> {
    Capability(&'a RuntimeCapability),
    Enhancement(EnhancementContract),
}

pub(super) async fn list(
    state: &RuntimeState,
    session: &SessionContext,
    request: Option<PaginatedRequestParams>,
) -> Result<ListToolsResult, ErrorData> {
    let after = request
        .and_then(|request| request.cursor)
        .map(cursor)
        .transpose()?;
    let (_, enhancements) = access::catalog(state, session)
        .await
        .map_err(|_| ErrorData::internal_error("读取增强目录失败", None))?;
    let invocation = access::capability_invocation(state, session, None)
        .map_err(|_| ErrorData::internal_error("读取原生能力目录失败", None))?;
    let native = invocation
        .catalog()
        .map_err(|_| ErrorData::internal_error("读取原生能力目录失败", None))?;
    // 只收集有界名称和契约引用，构造当前页时才复制 schema。
    let mut entries: Vec<_> = native
        .map(|contract| {
            (
                format!("runtime.{}", name(&contract.path)),
                Description::Capability(contract),
            )
        })
        .chain(
            enhancements
                .into_iter()
                .map(|contract| (name(&contract.path), Description::Enhancement(contract))),
        )
        .filter(|(name, _)| after.as_ref().is_none_or(|after| name > after))
        .collect();
    entries.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    let mut tools = Vec::new();
    let mut bytes = 0;
    let mut has_more = false;
    for (name, description) in entries {
        if tools.len() == 64 {
            has_more = true;
            break;
        }
        let tool = tool(name, description)?;
        let size = serde_json::to_vec(&tool)
            .map_err(|_| ErrorData::internal_error("编码能力目录失败", None))?
            .len();
        // 为游标及协议外层保留空间；一个超大契约不能造成永不前进的空分页。
        const PAGE_BYTES: usize = 256 * 1024 - 1024;
        if size > PAGE_BYTES {
            return Err(ErrorData::internal_error("能力契约超过目录页限额", None));
        }
        if bytes + size > PAGE_BYTES {
            has_more = true;
            break;
        }
        bytes += size;
        tools.push(tool);
    }
    let mut result = ListToolsResult::with_all_items(tools);
    if has_more {
        result.next_cursor = result
            .tools
            .last()
            .map(|tool| URL_SAFE_NO_PAD.encode(format!("1:{}", tool.name)));
    }
    Ok(result)
}

fn name(path: &str) -> String {
    path.trim_start_matches('/').replace('/', ".")
}

fn cursor(value: String) -> Result<String, ErrorData> {
    let invalid = || ErrorData::invalid_params("能力目录游标无效", None);
    if value.len() > 256 {
        return Err(invalid());
    }
    let bytes = URL_SAFE_NO_PAD.decode(value).map_err(|_| invalid())?;
    let decoded = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
    let name = decoded.strip_prefix("1:").ok_or_else(invalid)?;
    target(name)?;
    Ok(name.into())
}

fn tool(name: String, description: Description<'_>) -> Result<Tool, ErrorData> {
    let (description, input, output, revision, annotations) = match description {
        Description::Capability(contract) => (
            contract.description.clone(),
            contract.input_schema.clone(),
            contract.output_schema.clone(),
            false,
            Some(
                ToolAnnotations::default()
                    .read_only(contract.effect == CapabilityEffect::Read)
                    .idempotent(contract.effect != CapabilityEffect::Write),
            ),
        ),
        Description::Enhancement(contract) => (
            format!("调用运行时增强 {}", contract.path),
            contract.input_schema,
            contract.output_schema,
            true,
            None,
        ),
    };
    let mut tool = Tool::new(name, description, envelope("input", input, false)?);
    tool.output_schema = Some(Arc::new(envelope("output", output, revision)?));
    tool.annotations = annotations;
    Ok(tool)
}
