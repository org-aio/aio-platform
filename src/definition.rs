use std::{collections::BTreeMap, fmt, str::FromStr};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// 当前数据库程序协议版本。
pub const PROGRAM_SCHEMA_VERSION: u32 = 4;

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
}

const fn menu_enabled() -> bool {
    true
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation_model_id: Option<SymbolId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ModelIndexDefinition {
    pub id: SymbolId,
    pub fields: Vec<SymbolId>,
    pub purpose: IndexPurpose,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexPurpose {
    Filter,
    Sort,
    Relation,
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

/// 正式保存的页面只包含组件树、状态、数据源和事件绑定。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageDefinition {
    pub id: SymbolId,
    pub name: String,
    pub title: String,
    #[serde(default)]
    pub state: DefinitionState,
    pub root: ComponentNode,
    #[serde(default)]
    pub page_state: Vec<PageStateDefinition>,
    #[serde(default)]
    pub data_sources: Vec<DataSourceDefinition>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageStateDefinition {
    pub id: SymbolId,
    pub name: String,
    pub value_type: ValueType,
    #[serde(default)]
    pub initial_value: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComponentNode {
    pub id: SymbolId,
    pub component: String,
    #[serde(default)]
    pub state: DefinitionState,
    #[serde(default)]
    pub properties: BTreeMap<String, PropertyValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<PropertyValue>,
    #[serde(default)]
    pub events: BTreeMap<String, SymbolId>,
    #[serde(default)]
    pub children: Vec<ComponentNode>,
    #[serde(default)]
    pub style: ComponentStyle,
}

/// 页面只保存结构化设计约束，实际 class 由组件 Provider 决定。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComponentStyle {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spacing: Option<SpacingToken>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<SizeConstraint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<SizeConstraint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alignment: Option<Alignment>,
    #[serde(default)]
    pub responsive: BTreeMap<Breakpoint, ResponsiveStyle>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpacingToken {
    None,
    Xs,
    Sm,
    Md,
    Lg,
    Xl,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum SizeConstraint {
    Auto,
    Full,
    Content,
    Fraction(u8),
    Token(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Alignment {
    Start,
    Center,
    End,
    Stretch,
    SpaceBetween,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Breakpoint {
    Mobile,
    Tablet,
    Desktop,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResponsiveStyle {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spacing: Option<SpacingToken>,
}

/// 属性绑定只能引用稳定符号或事件值，不接受表达式字符串。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum PropertyValue {
    Literal {
        value: Value,
    },
    PageState {
        state_id: SymbolId,
    },
    DataSource {
        source_id: SymbolId,
        path: Vec<SymbolId>,
    },
    EventValue {
        name: String,
    },
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
pub struct DataSourceDefinition {
    pub id: SymbolId,
    pub name: String,
    pub function_id: SymbolId,
    #[serde(default)]
    pub parameters: BTreeMap<String, PropertyValue>,
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
    SetState {
        state_id: SymbolId,
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
    OpenDialog {
        component_id: SymbolId,
    },
    CloseDialog {
        component_id: SymbolId,
    },
    Notify {
        level: NotificationLevel,
    },
    Refresh {
        source_id: SymbolId,
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

/// 编译器只接受 Provider 暴露的结构化组件契约。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComponentCatalog {
    #[serde(default)]
    pub components: BTreeMap<String, ComponentContract>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComponentContract {
    pub canonical_id: String,
    #[serde(default)]
    pub properties: BTreeMap<String, ComponentPropertyContract>,
    #[serde(default)]
    pub events: BTreeMap<String, EventContract>,
    pub children: ChildrenConstraint,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComponentPropertyContract {
    pub value_type: ValueType,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub choices: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EventContract {
    #[serde(default)]
    pub payload: BTreeMap<String, ValueType>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChildrenConstraint {
    None,
    Any,
    Range { minimum: u32, maximum: u32 },
    Components { canonical_ids: Vec<String> },
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
