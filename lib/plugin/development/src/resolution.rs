use anyhow::{Result, ensure};
use az_plugin_manifest::RepositoryDependency;
use std::collections::{BTreeMap, BTreeSet};

/// 候选版本只含语言无关的依赖信息，网络、包缓存和本地工作区由调用方提供。
#[derive(Clone, Debug)]
pub struct DependencyCandidate {
    pub source: String,
    pub version: semver::Version,
    pub dependencies: Vec<RepositoryDependency>,
}

pub fn resolve_dependencies(
    target: &str,
    mut candidates: impl FnMut(&str) -> Result<Vec<DependencyCandidate>>,
) -> Result<Vec<DependencyCandidate>> {
    let mut solver = Solver {
        candidates: &mut candidates,
        cache: BTreeMap::new(),
        attempts: 0,
    };
    let selected = solver
        .search(
            BTreeMap::new(),
            vec![RepositoryDependency {
                git: target.into(),
                version: "*".into(),
            }],
        )?
        .ok_or_else(|| {
            anyhow::anyhow!("插件依赖存在版本冲突或循环，请检查锁定版本和 --with 覆盖")
        })?;
    order(&selected).ok_or_else(|| anyhow::anyhow!("插件依赖形成循环"))
}

struct Solver<'a, F> {
    candidates: &'a mut F,
    cache: BTreeMap<String, Vec<DependencyCandidate>>,
    attempts: usize,
}

impl<F: FnMut(&str) -> Result<Vec<DependencyCandidate>>> Solver<'_, F> {
    fn search(
        &mut self,
        selected: BTreeMap<String, DependencyCandidate>,
        mut pending: Vec<RepositoryDependency>,
    ) -> Result<Option<BTreeMap<String, DependencyCandidate>>> {
        ensure!(
            self.attempts < 4096 && selected.len() <= 128,
            "插件依赖解析超过复杂度限制"
        );
        self.attempts += 1;
        let Some(requirement) = pending.pop() else {
            return Ok(order(&selected).map(|_| selected));
        };
        let constraint = semver::VersionReq::parse(&requirement.version)?;
        if let Some(current) = selected.get(&requirement.git) {
            return if matches_requirement(&constraint, &current.version) {
                self.search(selected, pending)
            } else {
                Ok(None)
            };
        }
        if !self.cache.contains_key(&requirement.git) {
            let mut values = (self.candidates)(&requirement.git)?;
            ensure!(
                values.iter().all(|value| value.source == requirement.git),
                "依赖候选来源不匹配"
            );
            values.sort_by(|left, right| right.version.cmp(&left.version));
            self.cache.insert(requirement.git.clone(), values);
        }
        for value in self.cache[&requirement.git].clone() {
            if !matches_requirement(&constraint, &value.version) {
                continue;
            }
            let mut chosen = selected.clone();
            chosen.insert(value.source.clone(), value.clone());
            let mut requirements = pending.clone();
            requirements.extend(value.dependencies);
            if let Some(solution) = self.search(chosen, requirements)? {
                return Ok(Some(solution));
            }
        }
        Ok(None)
    }
}

fn order(selected: &BTreeMap<String, DependencyCandidate>) -> Option<Vec<DependencyCandidate>> {
    fn visit(
        source: &str,
        selected: &BTreeMap<String, DependencyCandidate>,
        visiting: &mut BTreeSet<String>,
        visited: &mut BTreeSet<String>,
        result: &mut Vec<DependencyCandidate>,
    ) -> Option<()> {
        if visited.contains(source) {
            return Some(());
        }
        if !visiting.insert(source.into()) {
            return None;
        }
        let candidate = selected.get(source)?;
        for dependency in &candidate.dependencies {
            visit(&dependency.git, selected, visiting, visited, result)?;
        }
        visiting.remove(source);
        visited.insert(source.into());
        result.push(candidate.clone());
        Some(())
    }
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    let mut result = Vec::new();
    for source in selected.keys() {
        visit(source, selected, &mut visiting, &mut visited, &mut result)?;
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn candidate(
        source: &str,
        version: &str,
        dependencies: &[(&str, &str)],
    ) -> DependencyCandidate {
        DependencyCandidate {
            source: source.into(),
            version: version.parse().unwrap(),
            dependencies: dependencies
                .iter()
                .map(|(git, version)| RepositoryDependency {
                    git: (*git).into(),
                    version: (*version).into(),
                })
                .collect(),
        }
    }
    #[test]
    fn finds_a_shared_version_instead_of_greedily_rejecting_a_diamond() -> Result<()> {
        let result = resolve_dependencies("root", |source| {
            Ok(match source {
                "root" => vec![candidate("root", "1.0.0", &[("left", "*"), ("right", "*")])],
                "left" => vec![candidate("left", "1.0.0", &[("shared", "<1.5")])],
                "right" => vec![candidate("right", "1.0.0", &[("shared", "^1")])],
                "shared" => vec![
                    candidate("shared", "1.9.0", &[]),
                    candidate("shared", "1.4.0", &[]),
                ],
                _ => vec![],
            })
        })?;
        assert_eq!(
            result
                .iter()
                .find(|value| value.source == "shared")
                .unwrap()
                .version
                .to_string(),
            "1.4.0"
        );
        assert_eq!(result.first().unwrap().source, "shared");
        Ok(())
    }
    #[test]
    fn rejects_cycles() {
        assert!(
            resolve_dependencies("root", |_| Ok(vec![candidate(
                "root",
                "1.0.0",
                &[("root", "*")]
            )]))
            .is_err()
        );
    }
}

/// `*` 接受所有已经发布成功的版本，包括自动交付生成的 dev 版本。
/// 有范围的要求继续遵循 semver 对预发布版本的显式选择规则。
pub fn matches_requirement(requirement: &semver::VersionReq, version: &semver::Version) -> bool {
    requirement.comparators.is_empty() || requirement.matches(version)
}
