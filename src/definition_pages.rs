/// 页面只声明需要的布局和数据，不指定组件实现。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageDefinition {
    pub id: SymbolId,
    pub name: String,
    pub title: String,
    #[serde(default)]
    pub state: DefinitionState,
    pub renderer: PageRendererDefinition,
    /// 页面声明的 REST 接口；内置布局接口由编译器推导。
    #[serde(default)]
    pub endpoints: Vec<PageEndpointDefinition>,
}

/// Studio 可见的页面 REST 接口元数据。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageEndpointDefinition {
    pub id: SymbolId,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
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

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PageRendererDefinition {
    #[default]
    ConventionFile,
    MenuTree,
    TreeTable {
        tree: TreeDefinition,
        table: TableDefinition,
    },
    CrudTable {
        table: TableDefinition,
    },
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

#[must_use]
pub fn function_nodes_can_connect(from: &FunctionNodeKind, to: &FunctionNodeKind) -> bool {
    !matches!(from, FunctionNodeKind::Fail { .. })
        && !matches!(
            to,
            FunctionNodeKind::Constant { .. } | FunctionNodeKind::Input { .. }
        )
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
    pub const fn as_str(self) -> &'static str {
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

pub(crate) fn data_identifier_is_valid(value: &str) -> bool {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_lowercase() || first == b'_')
        && bytes.all(|value| value.is_ascii_lowercase() || value.is_ascii_digit() || value == b'_')
}

pub(crate) fn endpoint_identifier_is_valid(value: &str) -> bool {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_lowercase() || first == b'_')
        && bytes.all(|value| value.is_ascii_alphanumeric() || value == b'_')
}

pub(crate) fn page_identifier_is_valid(value: &str) -> bool {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    first.is_ascii_lowercase()
        && bytes.all(|value| {
            value.is_ascii_lowercase() || value.is_ascii_digit() || matches!(value, b'_' | b'-')
        })
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

