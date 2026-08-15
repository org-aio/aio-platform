use crate::{ModelDefinition, PageRendererDefinition, SymbolId, TableDefinition, TreeDefinition};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum PageRendererKind {
    #[default]
    ConventionFile,
    MenuTree,
    TreeTable,
    CrudTable,
}

impl PageRendererKind {
    pub(crate) const fn title(self) -> &'static str {
        match self {
            Self::ConventionFile => "约定文件",
            Self::MenuTree => "程序菜单树",
            Self::TreeTable => "左树右表",
            Self::CrudTable => "增删改查表格",
        }
    }

    pub(crate) fn from_key(value: &str) -> Self {
        match value {
            "menu_tree" => Self::MenuTree,
            "tree_table" => Self::TreeTable,
            "crud_table" => Self::CrudTable,
            _ => Self::ConventionFile,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PageRendererDraft {
    pub(crate) kind: PageRendererKind,
    pub(crate) table_model_id: String,
    pub(crate) page_size: String,
    pub(crate) tree_model_id: String,
    pub(crate) tree_label_field_id: String,
    pub(crate) tree_parent_field_id: String,
    pub(crate) table_relation_field_id: String,
}

impl PageRendererDraft {
    pub(crate) fn from_definition(renderer: &PageRendererDefinition) -> Self {
        let (kind, table, tree) = match renderer {
            PageRendererDefinition::ConventionFile => (
                PageRendererKind::ConventionFile,
                TableDefinition::default(),
                TreeDefinition::default(),
            ),
            PageRendererDefinition::MenuTree => (
                PageRendererKind::MenuTree,
                TableDefinition::default(),
                TreeDefinition::default(),
            ),
            PageRendererDefinition::TreeTable { tree, table } => {
                (PageRendererKind::TreeTable, table.clone(), tree.clone())
            }
            PageRendererDefinition::CrudTable { table } => (
                PageRendererKind::CrudTable,
                table.clone(),
                TreeDefinition::default(),
            ),
        };
        Self {
            kind,
            table_model_id: optional_symbol_text(table.model_id),
            page_size: table.page_size.to_string(),
            tree_model_id: optional_symbol_text(tree.model_id),
            tree_label_field_id: optional_symbol_text(tree.label_field_id),
            tree_parent_field_id: optional_symbol_text(tree.parent_field_id),
            table_relation_field_id: optional_symbol_text(tree.table_relation_field_id),
        }
    }

    pub(crate) fn to_definition(
        &self,
        models: &[ModelDefinition],
    ) -> Result<PageRendererDefinition, Vec<String>> {
        if self.kind == PageRendererKind::ConventionFile {
            return Ok(PageRendererDefinition::ConventionFile);
        }
        if self.kind == PageRendererKind::MenuTree {
            return Ok(PageRendererDefinition::MenuTree);
        }

        let mut errors = Vec::new();
        let page_size = self
            .page_size
            .parse::<u32>()
            .ok()
            .filter(|value| (1..=200).contains(value));
        if page_size.is_none() {
            errors.push("每页条数必须是 1 到 200 的整数".to_owned());
        }
        let table_model = layout_model(&self.table_model_id, "请选择表格模型", models, &mut errors);
        let table = TableDefinition {
            model_id: table_model.map(|model| model.id),
            page_size: page_size.unwrap_or_default(),
        };
        if self.kind == PageRendererKind::CrudTable {
            return if errors.is_empty() {
                Ok(PageRendererDefinition::CrudTable { table })
            } else {
                Err(errors)
            };
        }

        let tree_model = layout_model(&self.tree_model_id, "请选择树模型", models, &mut errors);
        let label_field_id = layout_field(
            &self.tree_label_field_id,
            "请选择树标题字段",
            tree_model,
            true,
            &mut errors,
        );
        let parent_field_id = layout_field(
            &self.tree_parent_field_id,
            "树父级字段已失效",
            tree_model,
            false,
            &mut errors,
        );
        let table_relation_field_id = layout_field(
            &self.table_relation_field_id,
            "请选择表关联字段",
            table_model,
            true,
            &mut errors,
        );
        if errors.is_empty() {
            Ok(PageRendererDefinition::TreeTable {
                tree: TreeDefinition {
                    model_id: tree_model.map(|model| model.id),
                    label_field_id,
                    parent_field_id,
                    table_relation_field_id,
                },
                table,
            })
        } else {
            Err(errors)
        }
    }
}

fn optional_symbol_text(value: Option<SymbolId>) -> String {
    value.map(|id| id.to_string()).unwrap_or_default()
}

fn layout_model<'a>(
    value: &str,
    missing_message: &str,
    models: &'a [ModelDefinition],
    errors: &mut Vec<String>,
) -> Option<&'a ModelDefinition> {
    let Some(model_id) = SymbolId::parse(value).ok() else {
        errors.push(missing_message.to_owned());
        return None;
    };
    let Some(model) = models.iter().find(|model| model.id == model_id) else {
        errors.push(format!("模型 {model_id} 已不存在"));
        return None;
    };
    Some(model)
}

fn layout_field(
    value: &str,
    missing_message: &str,
    model: Option<&ModelDefinition>,
    required: bool,
    errors: &mut Vec<String>,
) -> Option<SymbolId> {
    if value.is_empty() && !required {
        return None;
    }
    let Some(field_id) = SymbolId::parse(value).ok() else {
        if model.is_some() {
            errors.push(missing_message.to_owned());
        }
        return None;
    };
    if model.is_some_and(|model| model.fields.iter().all(|field| field.id != field_id)) {
        errors.push(missing_message.to_owned());
        return None;
    }
    Some(field_id)
}

#[cfg(test)]
mod tests {
    use crate::{DefinitionState, FieldDefinition, FieldOptions, ModelAuditDefinition, ValueType};

    use super::*;

    fn model(name: &str, title: &str, field_names: &[&str]) -> ModelDefinition {
        ModelDefinition {
            id: SymbolId::new(),
            name: name.to_owned(),
            title: title.to_owned(),
            state: DefinitionState::Known,
            primary_key: crate::ModelPrimaryKeyDefinition::default(),
            fields: field_names
                .iter()
                .map(|name| FieldDefinition {
                    id: SymbolId::new(),
                    name: (*name).to_owned(),
                    title: (*name).to_owned(),
                    value_type: ValueType::Text,
                    state: DefinitionState::Known,
                    required: false,
                    options: FieldOptions::default(),
                    relation: None,
                })
                .collect(),
            indexes: Vec::new(),
            queries: Vec::new(),
            validations: Vec::new(),
            audit: ModelAuditDefinition::default(),
        }
    }

    #[test]
    fn opening_convention_page_keeps_saved_renderer() {
        let draft = PageRendererDraft::from_definition(&PageRendererDefinition::ConventionFile);

        assert_eq!(draft.kind, PageRendererKind::ConventionFile);
        assert_eq!(PageRendererKind::from_key("tree_table").title(), "左树右表");
        assert_eq!(
            draft.to_definition(&[]),
            Ok(PageRendererDefinition::ConventionFile)
        );
    }

    #[test]
    fn menu_tree_does_not_require_record_models() {
        let draft = PageRendererDraft::from_definition(&PageRendererDefinition::MenuTree);

        assert_eq!(draft.kind, PageRendererKind::MenuTree);
        assert_eq!(
            draft.to_definition(&[]),
            Ok(PageRendererDefinition::MenuTree)
        );
    }

    #[test]
    fn tree_table_requires_models_fields_and_valid_page_size() {
        let tree = model("department", "部门", &["name", "parent_id"]);
        let table = model("user", "用户", &["name", "department_id"]);
        let models = vec![tree.clone(), table.clone()];
        let mut draft = PageRendererDraft {
            kind: PageRendererKind::TreeTable,
            table_model_id: table.id.to_string(),
            page_size: "0".to_owned(),
            tree_model_id: tree.id.to_string(),
            tree_label_field_id: String::new(),
            tree_parent_field_id: String::new(),
            table_relation_field_id: String::new(),
        };

        let errors = draft
            .to_definition(&models)
            .expect_err("不完整树表草稿必须被拒绝");
        assert!(errors.iter().any(|error| error.contains("每页条数")));
        assert!(errors.iter().any(|error| error.contains("树标题字段")));
        assert!(errors.iter().any(|error| error.contains("表关联字段")));

        draft.page_size = "40".to_owned();
        draft.tree_label_field_id = tree.fields[0].id.to_string();
        draft.tree_parent_field_id = tree.fields[1].id.to_string();
        draft.table_relation_field_id = table.fields[1].id.to_string();
        let renderer = draft
            .to_definition(&models)
            .expect("完整树表草稿应当生成页面定义");
        assert!(matches!(
            renderer,
            PageRendererDefinition::TreeTable { table, .. }
                if table.model_id == Some(models[1].id) && table.page_size == 40
        ));
    }
}
