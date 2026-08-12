use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
};

use crate::{
    BooleanOperator, ChildCollection, CompareOperator, DefinitionState, DraftSnapshot, EffectKind,
    EndpointInputDefinition, EndpointInputLocation, EndpointOutputDefinition, FieldDefinition,
    FunctionDefinition, FunctionGraph, FunctionNode, FunctionNodeEditor, FunctionNodeKind,
    GraphEdge, GraphEntity, GraphPatch, GraphPatchBatch, MathOperator, MenuDefinition,
    ModelDefinition, ModelIndexDefinition, NotificationLevel, PageDefinition,
    PageEndpointDefinition, PageEndpointSource, PatchOrigin, PermissionDefinition, PortDefinition,
    PropertyValue, RestMethod, RouteDefinition, SymbolId, ValidationRule, ValidationRuleKind,
    ValueType, VibeRunAccepted, VibeRunRequest, VibeSessionSnapshot, validate_route_path,
};
use dioxus::prelude::*;
use serde_json::Value;

use crate::browser_http::{get_api, patch_api, post_api};
use crate::identifier_generation::{
    next_endpoint_path_parameter_name, next_function_node_name, normalize_endpoint_parameter_names,
    synchronize_path_parameter_names, unique_identifier_from_title,
};
use crate::page_endpoint_draft::validate_page_endpoint_draft;
use crate::page_renderer_draft::{PageRendererDraft, PageRendererKind};
use crate::studio_navigation::{
    ModelUsageSummary, StudioTab, definition_matches_search, delete_menu_patches,
    delete_page_patches, function_node_reference_count, function_port_reference_count,
    function_reference_count, model_usage_summary, page_menu_reference_count, permission_usage_map,
    preferred_draft_scene_id,
};
use az_admin_shell_core::identifier_from_title;
use az_ui_components::{
    agent_chat::{AgentChat, AgentChatMessage},
    badge::{Badge, BadgeVariant},
    button::{Button, ButtonSize, ButtonVariant},
    checkbox::{Checkbox, checkbox_is_checked, checkbox_state},
    collection_tree::{CollectionTree, CollectionTreeData, CollectionTreeItemContext},
    data_table::{
        DataTable, DataTableAlign, DataTableCellContext, DataTableColumn, DataTableEditContext,
        DataTableEditTrigger, DataTableFixed, DataTableRowTone, DataTableSpan,
    },
    dialog::{Dialog, DialogDescription, DialogTitle},
    input::Input,
    navigation_icon::{NavigationIcon, NavigationIconPicker, resolved_navigation_icon},
    select::{Select, SelectItem, SelectPlacement},
    spatial::{GraphCanvas, GraphNode, GraphNodeState, TreeIndent},
    textarea::Textarea,
};
use gloo_timers::future::TimeoutFuture;

mod endpoint_dialog;
mod endpoint_editor;
mod endpoint_panel;
mod field_dialog;
mod function_canvas;
mod function_definition_dialog;
mod function_edge_dialog;
mod function_node_dialog;
mod function_node_fields;
mod function_panel;
mod function_tables;
mod function_validation;
mod index_dialog;
mod menu_dialog;
mod menu_panel;
mod menu_table;
mod model_agent_chat;
mod model_audit;
mod model_fields;
mod model_grid;
mod model_panel;
mod model_tables;
mod page_dialog;
mod page_panel;
mod page_renderer;
mod patch_queue;
mod permission_panel;
mod query_dialog;
mod relation_dialog;
mod shell;
mod validation_dialog;

use endpoint_dialog::*;
use endpoint_editor::*;
use endpoint_panel::*;
use field_dialog::*;
use function_canvas::*;
use function_definition_dialog::*;
use function_edge_dialog::*;
use function_node_dialog::*;
use function_node_fields::*;
use function_panel::*;
use function_tables::*;
use function_validation::*;
use index_dialog::*;
use menu_dialog::*;
use menu_panel::*;
use menu_table::*;
use model_agent_chat::*;
use model_audit::*;
use model_fields::*;
use model_grid::*;
use model_panel::*;
use model_tables::*;
use page_dialog::*;
use page_panel::*;
use page_renderer::*;
use patch_queue::*;
use permission_panel::*;
use query_dialog::*;
use relation_dialog::*;
use validation_dialog::*;

pub(crate) use shell::ProgramMenuTreePage;
pub use shell::StudioPage;
