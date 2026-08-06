use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DataTableAlign {
    #[default]
    Start,
    Center,
    End,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DataTableFixed {
    #[default]
    None,
    Left,
    Right,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataTableColumn {
    pub key: String,
    pub title: String,
    pub width: u16,
    pub align: DataTableAlign,
    pub fixed: DataTableFixed,
    pub editable: bool,
    pub children: Vec<Self>,
}

impl DataTableColumn {
    pub fn leaf(key: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            title: title.into(),
            width: 160,
            align: DataTableAlign::Start,
            fixed: DataTableFixed::None,
            editable: false,
            children: Vec::new(),
        }
    }

    pub fn group(key: impl Into<String>, title: impl Into<String>, children: Vec<Self>) -> Self {
        Self {
            key: key.into(),
            title: title.into(),
            width: 0,
            align: DataTableAlign::Center,
            fixed: DataTableFixed::None,
            editable: false,
            children,
        }
    }

    pub fn width(mut self, width: u16) -> Self {
        self.width = width.max(48);
        self
    }

    pub fn align(mut self, align: DataTableAlign) -> Self {
        self.align = align;
        self
    }

    pub fn fixed(mut self, fixed: DataTableFixed) -> Self {
        self.fixed = fixed;
        self
    }

    pub fn editable(mut self) -> Self {
        self.editable = true;
        self
    }

    pub fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataTableSpan {
    pub row: usize,
    pub column_key: String,
    pub rowspan: usize,
    pub colspan: usize,
}

impl DataTableSpan {
    pub fn new(row: usize, column_key: impl Into<String>, rowspan: usize, colspan: usize) -> Self {
        Self {
            row,
            column_key: column_key.into(),
            rowspan,
            colspan,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedLeafColumn {
    pub column: DataTableColumn,
    pub left: Option<u32>,
    pub right: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataTableHeaderCell {
    pub key: String,
    pub title: String,
    pub level: usize,
    pub colspan: usize,
    pub rowspan: usize,
    pub width: u32,
    pub fixed: DataTableFixed,
    pub offset: Option<u32>,
    pub leaf_key: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DataTableBodyCell {
    pub visible: bool,
    pub rowspan: usize,
    pub colspan: usize,
}

impl Default for DataTableBodyCell {
    fn default() -> Self {
        Self {
            visible: true,
            rowspan: 1,
            colspan: 1,
        }
    }
}

pub fn resolve_leaf_columns(
    columns: &[DataTableColumn],
) -> Result<Vec<ResolvedLeafColumn>, String> {
    validate_columns(columns)?;
    let mut leaves = Vec::new();
    collect_leaves(columns, &mut leaves);
    validate_fixed_ranges(&leaves)?;

    let mut left_offset = 0_u32;
    let mut resolved = leaves
        .into_iter()
        .map(|column| {
            let left = (column.fixed == DataTableFixed::Left).then_some(left_offset);
            if left.is_some() {
                left_offset += u32::from(column.width);
            }
            ResolvedLeafColumn {
                column,
                left,
                right: None,
            }
        })
        .collect::<Vec<_>>();

    let mut right_offset = 0_u32;
    for column in resolved.iter_mut().rev() {
        if column.column.fixed == DataTableFixed::Right {
            column.right = Some(right_offset);
            right_offset += u32::from(column.column.width);
        }
    }
    Ok(resolved)
}

pub fn resolve_header_rows(
    columns: &[DataTableColumn],
    leaves: &[ResolvedLeafColumn],
) -> Result<Vec<Vec<DataTableHeaderCell>>, String> {
    let depth = column_depth(columns);
    if depth == 0 {
        return Err("表格至少需要一列".to_owned());
    }
    let leaf_map = leaves
        .iter()
        .map(|leaf| (leaf.column.key.as_str(), leaf))
        .collect::<BTreeMap<_, _>>();
    let mut rows = vec![Vec::new(); depth];
    collect_header_cells(columns, 0, depth, &leaf_map, &mut rows)?;
    Ok(rows)
}

pub fn resolve_body_grid(
    row_count: usize,
    leaves: &[ResolvedLeafColumn],
    spans: &[DataTableSpan],
) -> Result<Vec<Vec<DataTableBodyCell>>, String> {
    let column_index = leaves
        .iter()
        .enumerate()
        .map(|(index, leaf)| (leaf.column.key.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    let mut grid = vec![vec![DataTableBodyCell::default(); leaves.len()]; row_count];
    let mut occupied = vec![vec![false; leaves.len()]; row_count];

    for span in spans {
        if span.rowspan == 0 || span.colspan == 0 {
            return Err("合并单元格的 rowspan 与 colspan 必须大于 0".to_owned());
        }
        let Some(&column) = column_index.get(span.column_key.as_str()) else {
            return Err(format!("合并单元格引用了不存在的列: {}", span.column_key));
        };
        if span.row + span.rowspan > row_count || column + span.colspan > leaves.len() {
            return Err(format!(
                "合并单元格越界: row={}, column={}",
                span.row, span.column_key
            ));
        }
        if span.colspan > 1
            && leaves[column..column + span.colspan]
                .iter()
                .any(|leaf| leaf.column.fixed != DataTableFixed::None)
        {
            return Err("跨列合并不能覆盖固定列".to_owned());
        }
        for row in occupied.iter().skip(span.row).take(span.rowspan) {
            if row[column..column + span.colspan]
                .iter()
                .any(|value| *value)
            {
                return Err(format!(
                    "合并单元格发生重叠: row={}, column={}",
                    span.row, span.column_key
                ));
            }
        }
        grid[span.row][column] = DataTableBodyCell {
            visible: true,
            rowspan: span.rowspan,
            colspan: span.colspan,
        };
        for row in span.row..span.row + span.rowspan {
            for current_column in column..column + span.colspan {
                occupied[row][current_column] = true;
                if row != span.row || current_column != column {
                    grid[row][current_column].visible = false;
                }
            }
        }
    }
    Ok(grid)
}

fn validate_columns(columns: &[DataTableColumn]) -> Result<(), String> {
    if columns.is_empty() {
        return Err("表格至少需要一列".to_owned());
    }
    let mut keys = BTreeSet::new();
    validate_column_branch(columns, &mut keys)
}

fn validate_column_branch(
    columns: &[DataTableColumn],
    keys: &mut BTreeSet<String>,
) -> Result<(), String> {
    for column in columns {
        if column.key.trim().is_empty() {
            return Err("表格列 key 不能为空".to_owned());
        }
        if !keys.insert(column.key.clone()) {
            return Err(format!("表格列 key 重复: {}", column.key));
        }
        if column.is_leaf() && column.width == 0 {
            return Err(format!("表格分组列必须包含子列: {}", column.key));
        }
        if !column.is_leaf() {
            validate_column_branch(&column.children, keys)?;
        }
    }
    Ok(())
}

fn validate_fixed_ranges(columns: &[DataTableColumn]) -> Result<(), String> {
    let first_non_left = columns
        .iter()
        .position(|column| column.fixed != DataTableFixed::Left)
        .unwrap_or(columns.len());
    if columns[first_non_left..]
        .iter()
        .any(|column| column.fixed == DataTableFixed::Left)
    {
        return Err("固定左列必须连续位于最左侧".to_owned());
    }
    let right_start = columns
        .iter()
        .position(|column| column.fixed == DataTableFixed::Right)
        .unwrap_or(columns.len());
    if columns[right_start..]
        .iter()
        .any(|column| column.fixed != DataTableFixed::Right)
    {
        return Err("固定右列必须连续位于最右侧".to_owned());
    }
    Ok(())
}

fn collect_leaves(columns: &[DataTableColumn], leaves: &mut Vec<DataTableColumn>) {
    for column in columns {
        if column.is_leaf() {
            leaves.push(column.clone());
        } else {
            collect_leaves(&column.children, leaves);
        }
    }
}

fn column_depth(columns: &[DataTableColumn]) -> usize {
    columns
        .iter()
        .map(|column| {
            if column.is_leaf() {
                1
            } else {
                1 + column_depth(&column.children)
            }
        })
        .max()
        .unwrap_or(0)
}

fn collect_header_cells(
    columns: &[DataTableColumn],
    level: usize,
    depth: usize,
    leaf_map: &BTreeMap<&str, &ResolvedLeafColumn>,
    rows: &mut [Vec<DataTableHeaderCell>],
) -> Result<(), String> {
    for column in columns {
        let descendants = descendant_leaves(column, leaf_map)?;
        let fixed = common_fixed(&descendants);
        let offset = common_offset(fixed, &descendants);
        rows[level].push(DataTableHeaderCell {
            key: column.key.clone(),
            title: column.title.clone(),
            level,
            colspan: descendants.len(),
            rowspan: if column.is_leaf() { depth - level } else { 1 },
            width: descendants
                .iter()
                .map(|leaf| u32::from(leaf.column.width))
                .sum(),
            fixed,
            offset,
            leaf_key: column.is_leaf().then(|| column.key.clone()),
        });
        if !column.is_leaf() {
            collect_header_cells(&column.children, level + 1, depth, leaf_map, rows)?;
        }
    }
    Ok(())
}

fn descendant_leaves<'a>(
    column: &DataTableColumn,
    leaf_map: &'a BTreeMap<&str, &'a ResolvedLeafColumn>,
) -> Result<Vec<&'a ResolvedLeafColumn>, String> {
    if column.is_leaf() {
        return leaf_map
            .get(column.key.as_str())
            .copied()
            .map(|leaf| vec![leaf])
            .ok_or_else(|| format!("无法解析表格叶子列: {}", column.key));
    }
    let mut descendants = Vec::new();
    for child in &column.children {
        descendants.extend(descendant_leaves(child, leaf_map)?);
    }
    Ok(descendants)
}

fn common_fixed(columns: &[&ResolvedLeafColumn]) -> DataTableFixed {
    let Some(first) = columns.first().map(|column| column.column.fixed) else {
        return DataTableFixed::None;
    };
    if columns.iter().all(|column| column.column.fixed == first) {
        first
    } else {
        DataTableFixed::None
    }
}

fn common_offset(fixed: DataTableFixed, columns: &[&ResolvedLeafColumn]) -> Option<u32> {
    match fixed {
        DataTableFixed::None => None,
        DataTableFixed::Left => columns.iter().filter_map(|column| column.left).min(),
        DataTableFixed::Right => columns.iter().filter_map(|column| column.right).min(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn columns() -> Vec<DataTableColumn> {
        vec![
            DataTableColumn::leaf("index", "序号")
                .width(64)
                .fixed(DataTableFixed::Left),
            DataTableColumn::group(
                "identity",
                "接口",
                vec![
                    DataTableColumn::leaf("source", "来源").width(96),
                    DataTableColumn::leaf("path", "路径").width(280),
                ],
            ),
            DataTableColumn::group(
                "contract",
                "数据契约",
                vec![
                    DataTableColumn::leaf("inputs", "入参").width(80),
                    DataTableColumn::leaf("outputs", "响应").width(80),
                ],
            ),
            DataTableColumn::leaf("actions", "操作")
                .width(96)
                .fixed(DataTableFixed::Right),
        ]
    }

    #[test]
    fn tree_header_resolves_rowspan_colspan_and_fixed_offsets() {
        let leaves = resolve_leaf_columns(&columns()).expect("列布局应有效");
        let rows = resolve_header_rows(&columns(), &leaves).expect("树形表头应有效");

        assert_eq!(leaves.len(), 6);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0].rowspan, 2);
        assert_eq!(rows[0][1].colspan, 2);
        assert_eq!(rows[0][2].colspan, 2);
        assert_eq!(rows[0][3].fixed, DataTableFixed::Right);
        assert_eq!(rows[0][3].offset, Some(0));
    }

    #[test]
    fn body_span_hides_covered_cells() {
        let leaves = resolve_leaf_columns(&columns()).expect("列布局应有效");
        let grid = resolve_body_grid(3, &leaves, &[DataTableSpan::new(0, "source", 2, 2)])
            .expect("合并布局应有效");
        let source = leaves
            .iter()
            .position(|leaf| leaf.column.key == "source")
            .expect("应存在来源列");

        assert_eq!(grid[0][source].rowspan, 2);
        assert_eq!(grid[0][source].colspan, 2);
        assert!(grid[0][source].visible);
        assert!(!grid[0][source + 1].visible);
        assert!(!grid[1][source].visible);
        assert!(!grid[1][source + 1].visible);
        assert!(grid[2][source].visible);
    }

    #[test]
    fn overlapping_spans_are_rejected() {
        let leaves = resolve_leaf_columns(&columns()).expect("列布局应有效");
        let error = resolve_body_grid(
            3,
            &leaves,
            &[
                DataTableSpan::new(0, "source", 2, 1),
                DataTableSpan::new(1, "source", 2, 1),
            ],
        )
        .expect_err("重叠合并必须失败");

        assert!(error.contains("重叠"));
    }

    #[test]
    fn colspan_across_fixed_column_is_rejected() {
        let leaves = resolve_leaf_columns(&columns()).expect("列布局应有效");
        let error = resolve_body_grid(2, &leaves, &[DataTableSpan::new(0, "outputs", 1, 2)])
            .expect_err("不能跨固定列合并");

        assert!(error.contains("固定列"));
    }
}
