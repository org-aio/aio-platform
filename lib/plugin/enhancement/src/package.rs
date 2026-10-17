use std::{collections::BTreeMap, sync::Arc};

use anyhow::{Context, Result, ensure};
use az_plugin_contract::validate_enhancement_path;
use sha2::{Digest, Sha256};

use crate::{
    Enhancement, InvocationCapabilities, Package, ScriptContext, ScriptInvocation, ScriptProgram,
    ScriptRuntime,
};

/// 已完成语法检查和样例执行的候选版本；发布者负责持久化和原子激活。
#[derive(Clone)]
pub struct PreparedPackage {
    revision: String,
    package: Arc<Package>,
    pub(crate) compiled: Arc<BTreeMap<String, PreparedEnhancement>>,
}

pub(crate) struct PreparedEnhancement {
    pub program: Arc<dyn ScriptProgram>,
    pub input: az_plugin_contract::ValueSchema,
    pub output: az_plugin_contract::ValueSchema,
}

impl ScriptRuntime {
    /// 使用已装配的执行器编译候选并重放样例，成功后返回不可变的准备版本。
    pub fn prepare(&self, package: Package) -> Result<PreparedPackage> {
        self.validate()?;
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
            let input =
                az_plugin_contract::ValueSchema::compile(enhancement.input_schema.as_ref())?;
            let output =
                az_plugin_contract::ValueSchema::compile(enhancement.output_schema.as_ref())?;
            let program = self.compile(enhancement.language, &enhancement.source)?;
            let implementation = PreparedEnhancement {
                program,
                input,
                output,
            };
            compiled.insert(enhancement.path.clone(), implementation);
        }
        let prepared = PreparedPackage {
            revision: format!("{:x}", Sha256::digest(&encoded)),
            package: Arc::new(package),
            compiled: Arc::new(compiled),
        };
        prepared.validate_dependencies()?;
        prepared.check_examples(&prepared.package)?;
        Ok(prepared)
    }
}

impl PreparedPackage {
    pub fn revision(&self) -> &str {
        &self.revision
    }

    pub fn package(&self) -> &Package {
        &self.package
    }

    /// 固定版本和授权快照；截止时间取入口期限与宿主上限的较小值，不能由子调用重置。
    pub fn invocation(
        &self,
        context: ScriptContext,
        grants: Vec<String>,
        capabilities: InvocationCapabilities,
        deadline: Option<std::time::Instant>,
    ) -> ScriptInvocation {
        ScriptInvocation::new(self.clone(), context, grants, capabilities, deadline)
    }

    /// 返回整个依赖闭包，用于候选检查和调用前的权限校验。
    pub fn required_paths(&self, path: &str) -> Result<Vec<String>> {
        let mut result = Vec::new();
        self.visit_dependencies(path, &mut Vec::new(), &mut result)?;
        Ok(result)
    }

    /// 在任何脚本执行前校验完整依赖闭包的宿主能力，避免中途失败留下部分副作用。
    pub fn required_capabilities(&self, path: &str) -> Result<Vec<String>> {
        let required = self.required_paths(path)?;
        let paths: std::collections::BTreeSet<_> = self
            .package
            .enhancements
            .iter()
            .filter(|entry| required.contains(&entry.path))
            .flat_map(|entry| entry.capabilities.iter().cloned())
            .collect();
        Ok(paths.into_iter().collect())
    }

    fn validate_dependencies(&self) -> Result<()> {
        for enhancement in &self.package.enhancements {
            self.required_paths(&enhancement.path)?;
        }
        Ok(())
    }

    fn visit_dependencies(
        &self,
        path: &str,
        stack: &mut Vec<String>,
        result: &mut Vec<String>,
    ) -> Result<()> {
        ensure!(
            !stack.iter().any(|entry| entry == path),
            "增强依赖存在循环: {path}"
        );
        ensure!(
            stack.len() < crate::invocation::MAX_DEPTH,
            "增强依赖超过调用深度配额"
        );
        let enhancement = self
            .package
            .enhancements
            .iter()
            .find(|entry| entry.path == path)
            .with_context(|| format!("增强依赖不存在: {path}"))?;
        stack.push(path.into());
        for target in &enhancement.calls {
            self.visit_dependencies(target, stack, result)?;
        }
        stack.pop();
        if !result.iter().any(|entry| entry == path) {
            result.push(path.into());
        }
        Ok(())
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
                crate::value_schema::same(&candidate.input_schema, &enhancement.input_schema)
                    && crate::value_schema::same(
                        &candidate.output_schema,
                        &enhancement.output_schema
                    ),
                "升级不能改变已有 schema，请使用新版本路径: {}",
                enhancement.path
            );
            let preserved = enhancement.examples.iter().all(|example| {
                candidate
                    .examples
                    .iter()
                    .any(|candidate| same_example(example, candidate))
            });
            ensure!(preserved, "升级不能删除已有契约样例: {}", enhancement.path);
            for (index, example) in enhancement.examples.iter().enumerate() {
                let grants = self.required_paths(&enhancement.path)?;
                let caller = Arc::new(crate::capabilities::ExampleCaller::new(
                    example.capabilities.clone(),
                ));
                let capabilities = InvocationCapabilities {
                    grants: self.required_capabilities(&enhancement.path)?,
                    caller: Some(caller.clone()),
                };
                let invocation = self.invocation(context.clone(), grants, capabilities, None);
                let output = invocation.invoke(&enhancement.path, &example.input)?;
                caller.finish()?;
                ensure!(
                    crate::json_contract::equivalent(&output, &example.output),
                    "增强契约样例未通过: {} #{}",
                    enhancement.path,
                    index + 1
                );
            }
        }
        Ok(())
    }
}

fn same_example(left: &crate::Example, right: &crate::Example) -> bool {
    use crate::json_contract::equivalent;
    equivalent(&left.input, &right.input)
        && equivalent(&left.output, &right.output)
        && left.capabilities.len() == right.capabilities.len()
        && left
            .capabilities
            .iter()
            .zip(&right.capabilities)
            .all(|(left, right)| {
                left.path == right.path
                    && equivalent(&left.input, &right.input)
                    && equivalent(&left.output, &right.output)
            })
}

fn validate(enhancement: &Enhancement) -> Result<()> {
    validate_enhancement_path(&enhancement.path)?;
    let mut dependencies = std::collections::HashSet::new();
    ensure!(enhancement.calls.len() <= 16, "增强依赖数量超过配额");
    for path in &enhancement.calls {
        validate_enhancement_path(path)?;
        ensure!(dependencies.insert(path), "增强依赖重复: {path}");
    }
    let mut capabilities = std::collections::HashSet::new();
    ensure!(
        enhancement.capabilities.len() <= 32,
        "增强宿主能力数量超过配额"
    );
    for path in &enhancement.capabilities {
        validate_enhancement_path(path)?;
        ensure!(capabilities.insert(path), "增强宿主能力重复: {path}");
    }
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
        ensure!(example.capabilities.len() <= 64, "增强样例宿主调用超过配额");
        for call in &example.capabilities {
            validate_enhancement_path(&call.path)?;
        }
        ensure!(
            serde_json::to_vec(example)?.len() <= crate::MAX_VALUE_BYTES,
            "增强样例超过配额"
        );
    }
    Ok(())
}
