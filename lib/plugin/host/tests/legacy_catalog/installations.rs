use super::file::FileCatalog;
use anyhow::{Context, Result, ensure};
use az_plugin_host::runtime::{server::plugin_catalog::*, *};
use uuid::Uuid;

#[async_trait::async_trait]
impl PluginInstallations for FileCatalog {
    async fn has_plugins(&self, tenant: &str) -> Result<bool> {
        Ok(self
            .read()
            .await?
            .bindings
            .iter()
            .any(|b| b.tenant == tenant))
    }
    async fn source_id(&self, git: &str) -> Result<Option<String>> {
        Ok(self
            .read()
            .await?
            .revisions
            .iter()
            .find(|r| r.git == git)
            .map(|r| r.source.clone()))
    }
    async fn record_lifecycle_event(
        &self,
        tenant: &str,
        source: &str,
        _revision: Option<&str>,
        lifecycle: &str,
        detail: &str,
    ) -> Result<()> {
        self.change(|data| {
            data.event(tenant, source, lifecycle, detail);
            Ok(())
        })
        .await
    }
    async fn lifecycle_events(
        &self,
        tenant: &str,
        source: &str,
    ) -> Result<Vec<PluginLifecycleEvent>> {
        Ok(self
            .read()
            .await?
            .events
            .into_iter()
            .rev()
            .filter(|(t, s, _)| t == tenant && s == source)
            .take(20)
            .map(|(_, _, event)| event)
            .collect())
    }
    async fn catalog(
        &self,
        tenant: &str,
        label: &str,
        user: UserView,
        availability: &ExecutionAvailability,
    ) -> Result<RuntimeCatalog> {
        let data = self.read().await?;
        let mut catalog = RuntimeCatalog {
            tenant: TenantView {
                id: tenant.into(),
                label: label.into(),
            },
            user,
            pages: vec![],
            plugins: vec![],
            page_versions: Default::default(),
            hidden_pages: vec![],
            account_items: vec![],
            plugin_settings: vec![],
            session_context: String::new(),
            context: String::new(),
        };
        for binding in data.bindings.iter().filter(|b| b.tenant == tenant) {
            let revision = data.revision(&binding.source, &binding.revision)?;
            let manifest: az_plugin_manifest::PluginManifest =
                serde_json::from_value(revision.manifest.clone())?;
            let state = availability.state(revision.runtime, binding.enabled);
            if state == PluginState::Active {
                let pages = data.pages(tenant, revision);
                for action in manifest.subplugins.iter().flat_map(|s| &s.account_actions) {
                    let page = pages
                        .iter()
                        .find(|p| p.id == *action)
                        .context("账户页面缺失")?;
                    catalog.account_items.push(RuntimeAccountItem {
                        id: format!("{}:{action}", binding.source),
                        label: page.label.clone(),
                        icon: page.icon.clone(),
                        page_id: page.id.clone(),
                        required_permission: page.required_permission.clone(),
                    });
                }
                for page in &pages {
                    ensure!(!catalog.page_versions.contains_key(&page.id), "页面冲突");
                    catalog.page_versions.insert(
                        page.id.clone(),
                        serde_json::to_string(&(
                            &binding.source,
                            &binding.revision,
                            &binding.generation,
                        ))?,
                    );
                }
                catalog.pages.extend(pages);
            }
            catalog.plugins.push(InstalledPluginView {
                source_id: binding.source.clone(),
                git: revision.git.clone(),
                revision: revision.revision.clone(),
                runtime: revision.runtime,
                state,
                capabilities: manifest.capabilities,
            });
        }
        Ok(catalog)
    }
    async fn set_enabled(
        &self,
        tenant: &str,
        source: &str,
        enabled: bool,
        instance: Option<&ProcessInstance>,
    ) -> Result<()> {
        ensure!(instance.is_none(), "夹具不接受进程");
        self.change(|data| {
            let binding = data
                .bindings
                .iter_mut()
                .find(|b| b.tenant == tenant && b.source == source)
                .context("安装不存在")?;
            binding.enabled = enabled;
            binding.generation = Uuid::new_v4().to_string();
            data.event(
                tenant,
                source,
                if enabled { "activate" } else { "deactivate" },
                "切换状态",
            );
            Ok(())
        })
        .await
    }
    async fn uninstall(&self, tenant: &str, source: &str) -> Result<()> {
        self.change(|data| {
            data.binding(tenant, source)?;
            let revisions = data
                .revisions
                .iter()
                .filter(|r| r.source == source)
                .map(|r| r.id())
                .collect::<Vec<_>>();
            data.pages
                .retain(|p| p.tenant != tenant || !revisions.contains(&p.revision));
            data.bindings
                .retain(|b| b.tenant != tenant || b.source != source);
            data.event(tenant, source, "uninstall", "移除安装");
            Ok(())
        })
        .await
    }
    async fn rollback_target(&self, tenant: &str, source: &str) -> Result<BoundRuntime> {
        let data = self.read().await?;
        let binding = data.binding(tenant, source)?;
        Ok(data
            .revisions
            .iter()
            .rev()
            .find(|r| r.source == source && r.revision != binding.revision)
            .context("历史版本不存在")?
            .target())
    }
    async fn rollback_to(
        &self,
        tenant: &str,
        source: &str,
        target: &BoundRuntime,
        instance: Option<&ProcessInstance>,
    ) -> Result<()> {
        ensure!(instance.is_none(), "夹具不接受进程");
        self.change(|data| {
            ensure!(
                data.revision(source, &target.revision)?.id() == target.revision_id,
                "回滚来源不匹配"
            );
            let binding = data
                .bindings
                .iter_mut()
                .find(|b| b.tenant == tenant && b.source == source)
                .context("安装不存在")?;
            binding.excluded = Some(binding.revision.clone());
            binding.revision = target.revision.clone();
            binding.enabled = true;
            binding.generation = Uuid::new_v4().to_string();
            data.event(tenant, source, "rollback", "回滚版本");
            Ok(())
        })
        .await
    }
    async fn active_service(&self, tenant: &str, source: &str) -> Result<Option<ServiceBinding>> {
        let data = self.read().await?;
        let Some(binding) = data
            .bindings
            .iter()
            .find(|b| b.tenant == tenant && b.source == source && b.enabled)
        else {
            return Ok(None);
        };
        let revision = data.revision(source, &binding.revision)?;
        if revision.runtime == PluginRuntime::WasmComponent {
            Ok(Some(revision.service()?))
        } else {
            Ok(None)
        }
    }
    async fn active_page_binding(&self, tenant: &str, page: &str) -> Result<Option<PageBinding>> {
        let data = self.read().await?;
        let mut found = None;
        for binding in data
            .bindings
            .iter()
            .filter(|b| b.tenant == tenant && b.enabled)
        {
            let revision = data.revision(&binding.source, &binding.revision)?;
            let Some(page) = data
                .pages(tenant, revision)
                .into_iter()
                .find(|p| p.id == page)
            else {
                continue;
            };
            ensure!(found.is_none(), "页面冲突");
            let generation = data
                .pages
                .iter()
                .find(|p| p.tenant == tenant && p.revision == revision.id() && p.page == page.id)
                .map(|p| p.generation);
            found = Some(PageBinding {
                source_id: binding.source.clone(),
                activation_generation: binding.generation.clone(),
                revision_id: revision.id(),
                state_generation: generation,
                page,
                service: revision.service()?,
            });
        }
        Ok(found)
    }
    async fn bound_runtime(&self, tenant: &str, source: &str) -> Result<BoundRuntime> {
        let data = self.read().await?;
        Ok(data
            .revision(source, &data.binding(tenant, source)?.revision)?
            .target())
    }
    async fn active_wasm_revision(&self, tenant: &str, source: &str) -> Result<Option<String>> {
        Ok(self
            .active_service(tenant, source)
            .await?
            .map(|service| service.revision))
    }
    async fn active_process(&self, _tenant: &str, _source: &str) -> Result<Option<ProcessBinding>> {
        self.ensure_no_process().await?;
        Ok(None)
    }
    async fn enabled_process_targets(&self) -> Result<Vec<ProcessTarget>> {
        self.ensure_no_process().await?;
        Ok(vec![])
    }
    async fn stop_orphan_process_records(&self) -> Result<()> {
        self.ensure_no_process().await
    }
    async fn enabled_wasm_targets(&self) -> Result<Vec<WasmTarget>> {
        let data = self.read().await?;
        data.bindings
            .iter()
            .filter(|b| b.enabled)
            .filter_map(|binding| {
                let result = (|| {
                    let revision = data.revision(&binding.source, &binding.revision)?;
                    Ok::<_, anyhow::Error>((revision.runtime == PluginRuntime::WasmComponent).then(
                        || WasmTarget {
                            tenant_id: binding.tenant.clone(),
                            source_id: binding.source.clone(),
                            revision_id: revision.id(),
                            revision: revision.revision.clone(),
                            artifact: revision.artifact.clone(),
                        },
                    ))
                })();
                result.transpose()
            })
            .collect()
    }
    async fn verify_revision_pages(&self, revision: &str, pages: &[PageDefinition]) -> Result<()> {
        let data = self.read().await?;
        ensure!(
            data.revisions
                .iter()
                .find(|r| r.id() == revision)
                .context("版本不存在")?
                .pages
                == pages,
            "页面发生漂移"
        );
        Ok(())
    }
    async fn recover_process_instance(
        &self,
        _target: &ProcessTarget,
        _instance: &ProcessInstance,
    ) -> Result<()> {
        anyhow::bail!("夹具不接受进程")
    }
    async fn restore_process_instance(
        &self,
        _tenant: &str,
        _source: &str,
        _target: &BoundRuntime,
        _instance: &ProcessInstance,
    ) -> Result<()> {
        anyhow::bail!("夹具不接受进程")
    }
    async fn recover_wasm_instance(&self, target: &WasmTarget) -> Result<()> {
        self.change(|data| {
            let binding = data.binding(&target.tenant_id, &target.source_id)?;
            ensure!(
                binding.enabled && binding.revision == target.revision,
                "恢复目标已改变"
            );
            data.event(
                &target.tenant_id,
                &target.source_id,
                "recover",
                "恢复 Wasm 实例",
            );
            Ok(())
        })
        .await
    }
}
impl FileCatalog {
    async fn ensure_no_process(&self) -> Result<()> {
        ensure!(
            self.read()
                .await?
                .revisions
                .iter()
                .all(|r| r.runtime != PluginRuntime::Process),
            "夹具不能恢复进程"
        );
        Ok(())
    }
}
