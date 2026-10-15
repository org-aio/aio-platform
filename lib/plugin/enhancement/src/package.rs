use std::collections::BTreeMap;

use anyhow::{Context, Result, ensure};
use az_plugin_contract::validate_enhancement_path;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{Enhancement, Language, Package, ScriptContext};

enum Compiled {
    Rhai(rhai::AST),
    TypeScript(String),
}

/// 已完成语法检查和样例执行的候选版本；发布者负责持久化和原子激活。
pub struct PreparedPackage {
    revision: String,
    package: Package,
    compiled: BTreeMap<String, Compiled>,
}

impl PreparedPackage {
    pub fn prepare(package: Package) -> Result<Self> {
        let encoded = serde_json::to_vec(&package)?;
        ensure!(encoded.len() <= crate::MAX_PACKAGE_BYTES, "增强包超过配额");
        ensure!(package.schema_version == 1, "增强包协议版本不受支持");
        ensure!(
            !package.enhancements.is_empty() && package.enhancements.len() <= 16,
            "增强数量必须在 1 到 16 之间"
        );
        let mut compiled = BTreeMap::new();
        for enhancement in &package.enhancements {
            validate(enhancement)?;
            ensure!(
                !compiled.contains_key(&enhancement.path),
                "增强路径重复: {}",
                enhancement.path
            );
            let implementation = match enhancement.language {
                Language::Rhai => Compiled::Rhai(crate::rhai::compile(&enhancement.source)?),
                Language::TypeScript => {
                    Compiled::TypeScript(crate::typescript::compile(&enhancement.source)?)
                }
            };
            compiled.insert(enhancement.path.clone(), implementation);
        }
        let prepared = Self {
            revision: format!("{:x}", Sha256::digest(&encoded)),
            package,
            compiled,
        };
        prepared.check_examples(&prepared.package)?;
        Ok(prepared)
    }

    pub fn revision(&self) -> &str {
        &self.revision
    }

    pub fn package(&self) -> &Package {
        &self.package
    }

    /// 每次执行创建独立作用域，禁止在不同调用、插件或租户间共享脚本状态。
    pub fn invoke(&self, path: &str, input: &Value, context: &ScriptContext) -> Result<Value> {
        ensure!(
            serde_json::to_vec(input)?.len() <= crate::MAX_VALUE_BYTES,
            "增强输入超过配额"
        );
        let implementation = self
            .compiled
            .get(path)
            .context("运行时尚未提供该增强能力")?;
        let output = match implementation {
            Compiled::Rhai(ast) => crate::rhai::invoke(ast, input, context),
            Compiled::TypeScript(code) => crate::typescript::invoke(code, input, context),
        }
        .with_context(|| format!("执行增强失败: {path}"))?;
        ensure!(
            serde_json::to_vec(&output)?.len() <= crate::MAX_VALUE_BYTES,
            "增强输出超过配额"
        );
        Ok(output)
    }

    /// 升级重放旧版契约样例，拒绝删除已有路径或悄悄改变已验证的结果。
    pub fn check_examples(&self, previous: &Package) -> Result<()> {
        let context = ScriptContext {
            tenant_id: "validation".into(),
            source_id: "validation".into(),
        };
        for enhancement in &previous.enhancements {
            let candidate = self
                .package
                .enhancements
                .iter()
                .find(|candidate| candidate.path == enhancement.path)
                .with_context(|| format!("升级不能删除已有能力: {}", enhancement.path))?;
            ensure!(
                enhancement
                    .examples
                    .iter()
                    .all(|example| candidate.examples.contains(example)),
                "升级不能删除已有契约样例: {}",
                enhancement.path
            );
            for (index, example) in enhancement.examples.iter().enumerate() {
                let output = self.invoke(&enhancement.path, &example.input, &context)?;
                ensure!(
                    output == example.output,
                    "增强契约样例未通过: {} #{}",
                    enhancement.path,
                    index + 1
                );
            }
        }
        Ok(())
    }
}

fn validate(enhancement: &Enhancement) -> Result<()> {
    validate_enhancement_path(&enhancement.path)?;
    ensure!(
        !enhancement.source.trim().is_empty()
            && enhancement.source.len() <= crate::MAX_SOURCE_BYTES,
        "增强源码为空或超过配额"
    );
    ensure!(
        !enhancement.examples.is_empty() && enhancement.examples.len() <= 8,
        "每项增强必须提供 1 到 8 个契约样例"
    );
    for example in &enhancement.examples {
        ensure!(
            serde_json::to_vec(example)?.len() <= crate::MAX_VALUE_BYTES,
            "增强样例超过配额"
        );
    }
    Ok(())
}
