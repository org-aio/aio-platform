use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    str::FromStr,
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// 当前数据库程序协议版本。
pub const PROGRAM_SCHEMA_VERSION: u32 = 15;

/// 应用源码需要支持的客户端发布目标。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationTarget {
    Web,
    Desktop,
}

impl ApplicationTarget {
    #[must_use]
    pub const fn cargo_feature(self) -> &'static str {
        match self {
            Self::Web => "web",
            Self::Desktop => "desktop",
        }
    }
}

#[must_use]
pub fn default_application_targets() -> BTreeSet<ApplicationTarget> {
    [ApplicationTarget::Web, ApplicationTarget::Desktop]
        .into_iter()
        .collect()
}

/// 创建时分配且永不因改名、改路由而变化的符号身份。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SymbolId(Uuid);

impl SymbolId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn parse(value: &str) -> Result<Self> {
        Uuid::parse_str(value)
            .map(Self)
            .with_context(|| format!("无效的 SymbolId: {value}"))
    }
}

impl Default for SymbolId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SymbolId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for SymbolId {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::parse(value)
    }
}

/// Draft 中每个可达声明的完备状态。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DefinitionState {
    #[default]
    Known,
    Hole {
        expected: String,
    },
    Unresolved {
        reference: String,
    },
    Conflict {
        candidates: Vec<String>,
    },
    Invalid {
        reason: String,
    },
}

impl DefinitionState {
    #[must_use]
    pub fn is_known(&self) -> bool {
        matches!(self, Self::Known)
    }
}

/// 数据库中唯一正式程序的完整定义。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProgramDefinition {
    pub schema_version: u32,
    pub id: SymbolId,
    pub name: String,
    pub title: String,
    #[serde(default = "default_application_targets")]
    pub application_targets: BTreeSet<ApplicationTarget>,
    #[serde(default)]
    pub menus: Vec<MenuDefinition>,
    #[serde(default)]
    pub models: Vec<ModelDefinition>,
    #[serde(default)]
    pub pages: Vec<PageDefinition>,
    #[serde(default)]
    pub functions: Vec<FunctionDefinition>,
    #[serde(default)]
    pub routes: Vec<RouteDefinition>,
    #[serde(default)]
    pub permissions: Vec<PermissionDefinition>,
}

impl ProgramDefinition {
    #[must_use]
    pub fn empty(name: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            schema_version: PROGRAM_SCHEMA_VERSION,
            id: SymbolId::new(),
            name: name.into(),
            title: title.into(),
            application_targets: default_application_targets(),
            menus: Vec::new(),
            models: Vec::new(),
            pages: Vec::new(),
            functions: Vec::new(),
            routes: Vec::new(),
            permissions: Vec::new(),
        }
    }
}

/// 可递归嵌套的菜单声明，根节点在界面中表现为场景。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MenuDefinition {
    pub id: SymbolId,
    pub name: String,
    pub title: String,
    #[serde(default)]
    pub state: DefinitionState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_id: Option<SymbolId>,
    #[serde(default = "menu_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub children: Vec<MenuDefinition>,
    #[serde(default)]
    pub required_permissions: Vec<SymbolId>,
    #[serde(default)]
    pub row_actions: MenuRowActions,
}

const fn menu_enabled() -> bool {
    true
}

/// 表格行操作直接随菜单声明，不在页面内部重复配置。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct MenuRowActions {
    #[serde(default)]
    pub detail: MenuActionAccess,
    #[serde(default)]
    pub edit: MenuActionAccess,
    #[serde(default)]
    pub delete: MenuActionAccess,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MenuActionAccess {
    #[default]
    Hidden,
    Public,
    Permission {
        permission_id: SymbolId,
    },
}

/// 动态模型仍映射到 MetaModel、MetaField 与 JSONB DataRecord。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelDefinition {
    pub id: SymbolId,
    pub name: String,
    pub title: String,
    #[serde(default)]
    pub state: DefinitionState,
    pub primary_key: ModelPrimaryKeyDefinition,
    #[serde(default)]
    pub fields: Vec<FieldDefinition>,
    #[serde(default)]
    pub indexes: Vec<ModelIndexDefinition>,
    #[serde(default)]
    pub queries: Vec<ModelQueryDefinition>,
    #[serde(default)]
    pub validations: Vec<ModelValidationDefinition>,
    #[serde(default)]
    pub audit: ModelAuditDefinition,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ModelPrimaryKeyDefinition {
    pub generation: PrimaryKeyGeneration,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrimaryKeyGeneration {
    #[default]
    Uuid,
    AutoIncrement,
}

impl PrimaryKeyGeneration {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Uuid => "uuid",
            Self::AutoIncrement => "auto_increment",
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Uuid => "UUID",
            Self::AutoIncrement => "自增整数",
        }
    }

    #[must_use]
    pub const fn value_type(self) -> ValueType {
        match self {
            Self::Uuid => ValueType::Text,
            Self::AutoIncrement => ValueType::Integer,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FieldDefinition {
    pub id: SymbolId,
    pub name: String,
    pub title: String,
    pub value_type: ValueType,
    #[serde(default)]
    pub state: DefinitionState,
    #[serde(default)]
    pub required: bool,
    pub options: FieldOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation: Option<FieldRelation>,
}

/// 关系必须由两端字段共同定义，避免只保存目标模型而无法确定关联路径。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FieldRelation {
    pub kind: RelationKind,
    pub target_model_id: SymbolId,
    pub target_field_id: SymbolId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    OneToOne,
    ManyToOne,
    OneToMany,
    ManyToMany,
}

impl RelationKind {
    #[must_use]
    pub const fn opposite(self) -> Self {
        match self {
            Self::OneToOne => Self::OneToOne,
            Self::ManyToOne => Self::OneToMany,
            Self::OneToMany => Self::ManyToOne,
            Self::ManyToMany => Self::ManyToMany,
        }
    }

    #[must_use]
    pub const fn is_collection(self) -> bool {
        matches!(self, Self::OneToMany | Self::ManyToMany)
    }
}

/// 字段在列表、表单、交换与 AI 场景中的唯一行为定义。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FieldOptions {
    pub list_visible: bool,
    pub detail_visible: bool,
    pub form_visible: bool,
    pub form_editable: bool,
    pub filterable: bool,
    pub sortable: bool,
    pub unique: bool,
    pub excel_import: bool,
    pub excel_export: bool,
    pub ai_extract: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help_text: Option<String>,
    pub validation: FieldValidation,
}

impl Default for FieldOptions {
    fn default() -> Self {
        Self {
            list_visible: true,
            detail_visible: true,
            form_visible: true,
            form_editable: true,
            filterable: false,
            sortable: false,
            unique: false,
            excel_import: true,
            excel_export: true,
            ai_extract: true,
            default_value: None,
            placeholder: None,
            help_text: None,
            validation: FieldValidation::default(),
        }
    }
}

/// 可直接映射为 JSON Schema 与运行时校验规则的字段约束。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FieldValidation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_length: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_items: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_items: Option<u32>,
    #[serde(default)]
    pub unique_items: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ModelIndexDefinition {
    pub id: SymbolId,
    pub fields: Vec<SymbolId>,
    #[serde(default)]
    pub unique: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryConjunction {
    #[default]
    All,
    Any,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ModelQueryDefinition {
    pub id: SymbolId,
    pub name: String,
    pub title: String,
    #[serde(default)]
    pub conjunction: QueryConjunction,
    #[serde(default)]
    pub conditions: Vec<QueryCondition>,
}

/// 查询条件的值来自命名参数，禁止把查询表达式作为字符串保存。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QueryCondition {
    Field {
        field_id: SymbolId,
        operator: QueryOperator,
        parameter: String,
    },
    Relation {
        relation_field_id: SymbolId,
        target_field_id: SymbolId,
        operator: QueryOperator,
        parameter: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryOperator {
    Equals,
    NotEquals,
    Contains,
    StartsWith,
    EndsWith,
    GreaterThan,
    GreaterOrEqual,
    LessThan,
    LessOrEqual,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ModelValidationDefinition {
    pub id: SymbolId,
    pub message: String,
    pub rule: ModelValidationRule,
}

/// 模型可组合的审计角色；角色只声明语义，具体字段由 `field_id` 绑定。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelAuditDefinition {
    #[serde(default)]
    pub fields: Vec<ModelAuditField>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ModelAuditField {
    pub kind: AuditFieldKind,
    pub field_id: SymbolId,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditFieldKind {
    TenantId,
    CreatedAt,
    CreatedBy,
    UpdatedAt,
    UpdatedBy,
    Deleted,
    DeletedAt,
    DeletedBy,
    Version,
}

impl AuditFieldKind {
    #[must_use]
    pub const fn all() -> [Self; 9] {
        [
            Self::TenantId,
            Self::CreatedAt,
            Self::CreatedBy,
            Self::UpdatedAt,
            Self::UpdatedBy,
            Self::Deleted,
            Self::DeletedAt,
            Self::DeletedBy,
            Self::Version,
        ]
    }

    #[must_use]
    pub const fn default_name(self) -> &'static str {
        match self {
            Self::TenantId => "tenant_id",
            Self::CreatedAt => "created_at",
            Self::CreatedBy => "created_by",
            Self::UpdatedAt => "updated_at",
            Self::UpdatedBy => "updated_by",
            Self::Deleted => "deleted",
            Self::DeletedAt => "deleted_at",
            Self::DeletedBy => "deleted_by",
            Self::Version => "version",
        }
    }

    #[must_use]
    pub const fn default_title(self) -> &'static str {
        match self {
            Self::TenantId => "租户",
            Self::CreatedAt => "创建时间",
            Self::CreatedBy => "创建人",
            Self::UpdatedAt => "更新时间",
            Self::UpdatedBy => "更新人",
            Self::Deleted => "逻辑删除",
            Self::DeletedAt => "删除时间",
            Self::DeletedBy => "删除人",
            Self::Version => "版本号",
        }
    }

    #[must_use]
    pub const fn default_value_type(self) -> ValueType {
        match self {
            Self::CreatedAt | Self::UpdatedAt | Self::DeletedAt => ValueType::TimestampMs,
            Self::Deleted => ValueType::Boolean,
            Self::Version => ValueType::Integer,
            Self::TenantId | Self::CreatedBy | Self::UpdatedBy | Self::DeletedBy => ValueType::Text,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::TenantId => "租户",
            Self::CreatedAt => "创建时间",
            Self::CreatedBy => "创建人",
            Self::UpdatedAt => "更新时间",
            Self::UpdatedBy => "更新人",
            Self::Deleted => "逻辑删除",
            Self::DeletedAt => "删除时间",
            Self::DeletedBy => "删除人",
            Self::Version => "版本号",
        }
    }
}

/// 模型级校验覆盖字段之间的依赖，不与单字段格式校验混在一起。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelValidationRule {
    FieldsRequiredTogether {
        field_ids: Vec<SymbolId>,
    },
    AtLeastOneRequired {
        field_ids: Vec<SymbolId>,
    },
    RequiredWhenPresent {
        field_id: SymbolId,
        when_field_id: SymbolId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ValueType {
    Any,
    Null,
    Boolean,
    Integer,
    Decimal,
    Text,
    TimestampMs,
    File,
    Object { model_id: SymbolId },
    List { item: Box<ValueType> },
    Optional { value: Box<ValueType> },
}

include!("pages.rs");

/// Capability 目录是编译期链接白名单，不包含可执行实现。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CapabilityCatalog {
    #[serde(default)]
    pub capabilities: BTreeMap<String, CapabilityContract>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CapabilityContract {
    pub canonical_id: String,
    #[serde(default)]
    pub operations: BTreeMap<String, CapabilityOperationContract>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CapabilityOperationContract {
    #[serde(default)]
    pub inputs: BTreeMap<String, ValueType>,
    #[serde(default)]
    pub outputs: BTreeMap<String, ValueType>,
    #[serde(default)]
    pub effects: Vec<EffectKind>,
}

pub(crate) fn validate_route_path(path: &str) -> Result<()> {
    if !path.starts_with('/') {
        bail!("路由必须以 / 开头: {path}");
    }
    if path.contains("//") || path.contains("..") || path.contains('?') || path.contains('#') {
        bail!("路由包含禁止片段: {path}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::PageEndpointDefinition;

    #[test]
    fn endpoint_display_title_is_derived_from_rest_path() -> anyhow::Result<()> {
        let endpoint: PageEndpointDefinition = serde_json::from_value(serde_json::json!({
            "id": "5cbf910c-05af-4537-94d3-673c3b4c444b",
            "title": "",
            "description": "批量停用资产",
            "method": "POST",
            "path": "/api/assets/batch-disable",
            "inputs": [],
            "outputs": []
        }))?;
        let value = serde_json::to_value(&endpoint)?;

        assert_eq!(endpoint.display_title(), "batch disable");
        assert!(value.get("implementation").is_none());
        Ok(())
    }
}
