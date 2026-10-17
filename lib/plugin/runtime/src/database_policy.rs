use anyhow::{Context, ensure};
use anyhow::{Result, bail};
use sqlparser::ast::{Expr, TableFactor, Visit, Visitor, visit_expressions, visit_relations};
use sqlparser::{ast::Statement, dialect::PostgreSqlDialect, parser::Parser};
use std::ops::ControlFlow;
pub(crate) fn validate(statement: &str) -> Result<()> {
    ensure!(statement.len() <= 64 * 1024, "SQL 语句超过配额");
    let statements =
        Parser::parse_sql(&PostgreSqlDialect {}, statement).context("解析插件 SQL 失败")?;
    ensure!(statements.len() == 1, "每次调用只允许一条 SQL");
    ensure!(
        matches!(
            &statements[0],
            Statement::Query(_)
                | Statement::Insert(_)
                | Statement::Update { .. }
                | Statement::Delete(_)
        ),
        "业务请求只允许查询和数据写入，事务与结构迁移由宿主管理"
    );
    validate_nodes(&statements[0])
}

pub(crate) fn validate_nodes(statement: &impl Visit) -> Result<()> {
    struct Tables;
    impl Visitor for Tables {
        type Break = &'static str;
        fn pre_visit_table_factor(&mut self, table: &TableFactor) -> ControlFlow<Self::Break> {
            match table {
                TableFactor::Table { args: None, .. }
                | TableFactor::Derived { .. }
                | TableFactor::NestedJoin { .. } => ControlFlow::Continue(()),
                _ => ControlFlow::Break("不能使用表函数或外部数据源"),
            }
        }
    }
    if let ControlFlow::Break(message) = statement.visit(&mut Tables) {
        bail!(message);
    }
    let relations = visit_relations(statement, |relation| {
        let name = relation.to_string().to_ascii_lowercase();
        if relation.0.len() != 1
            || name.trim_matches('"').starts_with("pg_")
            || name == "information_schema"
        {
            ControlFlow::Break("不能引用其他 schema 或系统表")
        } else {
            ControlFlow::Continue(())
        }
    });
    if let ControlFlow::Break(message) = relations {
        bail!(message);
    }
    let expressions = visit_expressions(statement, |expression| {
        if let Expr::Function(function) = expression {
            let name = function.name.to_string().to_ascii_lowercase();
            if !matches!(
                name.as_str(),
                "count"
                    | "now"
                    | "sum"
                    | "min"
                    | "max"
                    | "avg"
                    | "lower"
                    | "upper"
                    | "length"
                    | "char_length"
                    | "coalesce"
                    | "nullif"
                    | "abs"
                    | "round"
                    | "concat"
                    | "substring"
                    | "trim"
                    | "jsonb_build_object"
                    | "jsonb_build_array"
                    | "jsonb_agg"
            ) {
                return ControlFlow::Break("SQL 函数未列入受限能力集合");
            }
        }
        ControlFlow::Continue(())
    });
    if let ControlFlow::Break(message) = expressions {
        bail!(message);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn rejects_transaction_control_and_structure_changes() {
        for sql in [
            "COMMIT",
            "ROLLBACK",
            "SET ROLE admin",
            "CREATE TABLE x(id int)",
            "SELECT 1; DELETE FROM x",
            "COPY x TO '/tmp/x'",
            "SELECT set_config('statement_timeout', '0', false)",
            "SELECT * FROM set_config('statement_timeout', '0', false)",
            "SELECT * FROM other_tenant.counter",
            "SELECT * FROM pg_authid",
        ] {
            assert!(super::validate(sql).is_err(), "{sql}");
        }
        assert!(super::validate("SELECT 'a;b', $1::text").is_ok());
        assert!(super::validate("SELECT now()").is_ok());
        assert!(super::validate("UPDATE counter SET count=count+1 RETURNING count").is_ok());
    }
}
