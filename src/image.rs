use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    Breakpoint, ComponentStyle, EffectKind, MenuDefinition, PermissionDefinition, PropertyValue,
    SymbolId, ValueType,
};

/// bytecode 缓存目标，决定产物可在哪一侧执行。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageTarget {
    Server,
    WebAssembly,
    Universal,
}

impl ImageTarget {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::WebAssembly => "wasm",
            Self::Universal => "universal",
        }
    }
}

/// 可序列化、可缓存且与 Dioxus Element 解耦的发布产物。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApplicationImage {
    pub schema_version: u32,
    pub compiler_version: String,
    pub content_hash: String,
    pub application_id: SymbolId,
    pub name: String,
    pub title: String,
    pub revision_id: String,
    pub target: ImageTarget,
    pub menus: Vec<MenuDefinition>,
    pub permissions: Vec<PermissionDefinition>,
    pub pages: BTreeMap<SymbolId, RenderPlan>,
    pub client_functions: BTreeMap<SymbolId, BytecodeSegment>,
    pub server_functions: BTreeMap<SymbolId, BytecodeSegment>,
    pub models: BTreeMap<SymbolId, CompiledModel>,
    pub routes: Vec<CompiledRoute>,
    pub dependencies: BTreeMap<SymbolId, Vec<SymbolId>>,
}

impl ApplicationImage {
    pub fn encode(&self) -> anyhow::Result<Vec<u8>> {
        serde_json::to_vec(self).map_err(Into::into)
    }

    pub fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        serde_json::from_slice(bytes).map_err(Into::into)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenderPlan {
    pub page_id: SymbolId,
    pub name: String,
    pub title: String,
    pub root: RenderNode,
    pub page_state: BTreeMap<SymbolId, Value>,
    pub data_sources: Vec<CompiledDataSource>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenderNode {
    pub id: SymbolId,
    pub component: String,
    pub properties: BTreeMap<String, PropertyValue>,
    pub content: Option<PropertyValue>,
    pub events: BTreeMap<String, SymbolId>,
    pub children: Vec<RenderNode>,
    pub style: ComponentStyle,
    pub responsive_visibility: BTreeMap<Breakpoint, bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledDataSource {
    pub id: SymbolId,
    pub name: String,
    pub function_id: SymbolId,
    pub parameters: BTreeMap<String, PropertyValue>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompiledRoute {
    pub id: SymbolId,
    pub name: String,
    pub path: String,
    pub page_id: SymbolId,
    pub required_permissions: Vec<SymbolId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompiledModel {
    pub id: SymbolId,
    pub name: String,
    pub field_slots: BTreeMap<SymbolId, u32>,
    pub field_types: BTreeMap<u32, ValueType>,
    pub field_names: BTreeMap<u32, String>,
    pub expression_indexes: Vec<CompiledExpressionIndex>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompiledExpressionIndex {
    pub fields: Vec<u32>,
    pub expression: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BytecodeSegment {
    pub id: SymbolId,
    pub name: String,
    pub input_ports: BTreeMap<SymbolId, String>,
    pub effects: Vec<EffectKind>,
    pub instructions: Vec<BytecodeInstruction>,
    pub constants: Vec<Value>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SegmentInvocationRequest {
    #[serde(default)]
    pub inputs: BTreeMap<SymbolId, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SegmentInvocationResult {
    pub value: Value,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BytecodeInstruction {
    pub node_id: SymbolId,
    pub input_slots: BTreeMap<String, u32>,
    pub output_slot: Option<u32>,
    pub instruction: Instruction,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "opcode", rename_all = "snake_case")]
pub enum Instruction {
    LoadConstant {
        slot: u32,
        constant: u32,
    },
    LoadInput {
        slot: u32,
        port_id: SymbolId,
    },
    MakeObject {
        slot: u32,
        fields: Vec<SymbolId>,
    },
    MakeList {
        slot: u32,
        count: u32,
    },
    ReadField {
        slot: u32,
        field_id: SymbolId,
    },
    Format {
        slot: u32,
        template: String,
        count: u32,
    },
    Compare {
        slot: u32,
        operator: String,
    },
    Boolean {
        slot: u32,
        operator: String,
    },
    Math {
        slot: u32,
        operator: String,
    },
    Branch {
        condition_slot: u32,
    },
    ForEach {
        max_items: u32,
        body_function_id: SymbolId,
    },
    SetState {
        state_id: SymbolId,
    },
    ValidateForm {
        rule_count: u32,
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
    Confirm,
    OpenDialog {
        component_id: SymbolId,
    },
    CloseDialog {
        component_id: SymbolId,
    },
    Notify {
        level: String,
    },
    Refresh {
        source_id: SymbolId,
    },
    InvokeCapability {
        capability_id: String,
        operation: String,
    },
    InvokeServerSegment {
        segment_id: SymbolId,
        input_port: SymbolId,
    },
    Return,
    Fail {
        code: String,
    },
}
