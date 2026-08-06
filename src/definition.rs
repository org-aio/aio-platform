use std::{collections::BTreeMap, fmt, str::FromStr};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// 当前数据库程序协议版本。
pub const PROGRAM_SCHEMA_VERSION: u32 = 10;

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
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DefinitionState {
    Known,
    Hole { expected: String },
    Unresolved { reference: String },
    Conflict { candidates: Vec<String> },
    Invalid { reason: String },
}

impl Default for DefinitionState {
    fn default() -> Self {
        Self::Known
    }
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

/// 页面只声明需要的布局和数据，不指定组件实现。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageDefinition {
    pub id: SymbolId,
    pub name: String,
    pub title: String,
    #[serde(default)]
    pub state: DefinitionState,
    pub renderer: PageRendererDefinition,
    /// 页面声明的自定义 REST 接口；内置布局接口由编译器推导。
    #[serde(default)]
    pub endpoints: Vec<PageEndpointDefinition>,
}

/// 页面作为前端消费者所需的自定义 REST 接口。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageEndpointDefinition {
    pub id: SymbolId,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default)]
    pub state: DefinitionState,
    pub method: RestMethod,
    pub path: String,
    #[serde(default)]
    pub inputs: Vec<EndpointInputDefinition>,
    #[serde(default)]
    pub outputs: Vec<EndpointOutputDefinition>,
}

impl PageEndpointDefinition {
    #[must_use]
    pub fn display_title(&self) -> String {
        let title = self.title.trim();
        if !title.is_empty() {
            return title.to_owned();
        }

        self.path
            .trim_end_matches('/')
            .rsplit('/')
            .find(|segment| {
                !segment.is_empty() && !(segment.starts_with('{') && segment.ends_with('}'))
            })
            .map(|segment| segment.replace(['-', '_'], " "))
            .filter(|segment| !segment.is_empty())
            .unwrap_or_else(|| "REST 接口".to_owned())
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum RestMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl RestMethod {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EndpointInputDefinition {
    pub id: SymbolId,
    pub name: String,
    pub title: String,
    pub location: EndpointInputLocation,
    pub value_type: ValueType,
    #[serde(default)]
    pub required: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointInputLocation {
    Path,
    Query,
    Header,
    Body,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EndpointOutputDefinition {
    pub id: SymbolId,
    pub name: String,
    pub title: String,
    pub value_type: ValueType,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PageRendererDefinition {
    ConventionFile,
    TreeTable {
        tree: TreeDefinition,
        table: TableDefinition,
    },
    CrudTable {
        table: TableDefinition,
    },
}

impl Default for PageRendererDefinition {
    fn default() -> Self {
        Self::ConventionFile
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TableDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<SymbolId>,
    #[serde(default = "default_page_size")]
    pub page_size: u32,
}

const fn default_page_size() -> u32 {
    20
}

impl Default for TableDefinition {
    fn default() -> Self {
        Self {
            model_id: None,
            page_size: default_page_size(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct TreeDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<SymbolId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_field_id: Option<SymbolId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_field_id: Option<SymbolId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_relation_field_id: Option<SymbolId>,
}

/// 属性绑定只能引用稳定符号或事件值，不接受表达式字符串。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum PropertyValue {
    Literal { value: Value },
    EventValue { name: String },
}

impl PropertyValue {
    #[must_use]
    pub fn text(value: impl Into<String>) -> Self {
        Self::Literal {
            value: Value::String(value.into()),
        }
    }

    #[must_use]
    pub fn number(value: impl Into<serde_json::Number>) -> Self {
        Self::Literal {
            value: Value::Number(value.into()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FunctionDefinition {
    pub id: SymbolId,
    pub name: String,
    pub title: String,
    #[serde(default)]
    pub state: DefinitionState,
    #[serde(default)]
    pub inputs: Vec<PortDefinition>,
    #[serde(default)]
    pub outputs: Vec<PortDefinition>,
    pub graph: FunctionGraph,
    #[serde(default)]
    pub required_permissions: Vec<SymbolId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PortDefinition {
    pub id: SymbolId,
    pub name: String,
    pub value_type: ValueType,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FunctionGraph {
    #[serde(default)]
    pub nodes: Vec<FunctionNode>,
    #[serde(default)]
    pub edges: Vec<GraphEdge>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FunctionNode {
    pub id: SymbolId,
    pub name: String,
    #[serde(default)]
    pub state: DefinitionState,
    #[serde(default)]
    pub editor: FunctionNodeEditor,
    pub kind: FunctionNodeKind,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct FunctionNodeEditor {
    pub x: i32,
    pub y: i32,
}

/// 业务完备但刻意不图灵完备的函数节点集合。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FunctionNodeKind {
    Constant {
        value: Value,
        value_type: ValueType,
    },
    Input {
        port_id: SymbolId,
    },
    Output {
        port_id: SymbolId,
    },
    Object {
        fields: BTreeMap<SymbolId, SymbolId>,
    },
    List {
        items: Vec<SymbolId>,
    },
    FieldAccess {
        object: SymbolId,
        field_id: SymbolId,
    },
    Format {
        template: String,
        values: Vec<SymbolId>,
    },
    Compare {
        operator: CompareOperator,
    },
    Boolean {
        operator: BooleanOperator,
    },
    Math {
        operator: MathOperator,
    },
    Condition,
    ForEach {
        max_items: u32,
        body_function_id: SymbolId,
    },
    ValidateForm {
        rules: Vec<ValidationRule>,
    },
    CreateRecord {
        model_id: SymbolId,
    },
    ReadRecord {
        model_id: SymbolId,
    },
    UpdateRecord {
        model_id: SymbolId,
    },
    DeleteRecord {
        model_id: SymbolId,
    },
    QueryRecords {
        model_id: SymbolId,
        limit: u32,
    },
    Navigate {
        route_id: SymbolId,
    },
    Confirm {
        message: PropertyValue,
    },
    Notify {
        level: NotificationLevel,
    },
    Return,
    Fail {
        code: String,
    },
    Capability {
        capability_id: String,
        operation: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareOperator {
    Equal,
    NotEqual,
    Greater,
    GreaterOrEqual,
    Less,
    LessOrEqual,
    Contains,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BooleanOperator {
    And,
    Or,
    Not,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MathOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ValidationRule {
    pub field_id: SymbolId,
    pub rule: ValidationRuleKind,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ValidationRuleKind {
    Required,
    MinLength { value: u32 },
    MaxLength { value: u32 },
    Minimum { value: f64 },
    Maximum { value: f64 },
    Pattern { name: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationLevel {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub id: SymbolId,
    pub from_node: SymbolId,
    pub from_port: String,
    pub to_node: SymbolId,
    pub to_port: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RouteDefinition {
    pub id: SymbolId,
    pub name: String,
    pub path: String,
    pub page_id: SymbolId,
    #[serde(default)]
    pub state: DefinitionState,
    #[serde(default)]
    pub required_permissions: Vec<SymbolId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PermissionDefinition {
    pub id: SymbolId,
    pub name: String,
    pub title: String,
    #[serde(default)]
    pub allowed_effects: Vec<EffectKind>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectKind {
    ClientState,
    Navigation,
    UserPrompt,
    DatabaseRead,
    DatabaseWrite,
    Secret,
    Capability,
}

impl EffectKind {
    #[must_use]
    pub const fn all() -> [Self; 7] {
        [
            Self::ClientState,
            Self::Navigation,
            Self::UserPrompt,
            Self::DatabaseRead,
            Self::DatabaseWrite,
            Self::Secret,
            Self::Capability,
        ]
    }

    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::ClientState => "client_state",
            Self::Navigation => "navigation",
            Self::UserPrompt => "user_prompt",
            Self::DatabaseRead => "database_read",
            Self::DatabaseWrite => "database_write",
            Self::Secret => "secret",
            Self::Capability => "capability",
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ClientState => "客户端状态",
            Self::Navigation => "页面跳转",
            Self::UserPrompt => "用户交互",
            Self::DatabaseRead => "数据读取",
            Self::DatabaseWrite => "数据写入",
            Self::Secret => "机密访问",
            Self::Capability => "能力调用",
        }
    }
}

/// 权限标识采用 `领域:动作` 的稳定格式，例如 `asset:read`。
pub(crate) fn permission_identifier_is_valid(value: &str) -> bool {
    let mut segments = value.split(':');
    let Some(first) = segments.next() else {
        return false;
    };
    if !permission_identifier_segment_is_valid(first) {
        return false;
    }
    let mut has_action = false;
    for segment in segments {
        if !permission_identifier_segment_is_valid(segment) {
            return false;
        }
        has_action = true;
    }
    has_action
}

fn permission_identifier_segment_is_valid(value: &str) -> bool {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    first.is_ascii_lowercase()
        && bytes.all(|value| {
            value.is_ascii_lowercase() || value.is_ascii_digit() || matches!(value, b'_' | b'-')
        })
}

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
    fn endpoint_identity_is_derived_from_rest_path() -> anyhow::Result<()> {
        let endpoint: PageEndpointDefinition = serde_json::from_value(serde_json::json!({
            "id": "5cbf910c-05af-4537-94d3-673c3b4c444b",
            "name": "legacy_endpoint",
            "title": "",
            "intent": "旧的生成需求",
            "method": "POST",
            "path": "/api/assets/batch-disable",
            "inputs": [],
            "outputs": []
        }))?;
        let value = serde_json::to_value(&endpoint)?;

        assert_eq!(endpoint.display_title(), "batch disable");
        assert!(
            !value
                .as_object()
                .is_some_and(|object| object.contains_key("name"))
        );
        assert!(
            !value
                .as_object()
                .is_some_and(|object| object.contains_key("intent"))
        );
        Ok(())
    }
}
