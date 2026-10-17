use super::{Components, services};
use anyhow::Result;

impl Components {
    pub async fn permissions(&self, tenant: &str) -> Result<Vec<String>> {
        let rows = self.catalog.installations(Some(tenant), true).await?;
        let active = self.active.read().await;
        let mut permissions = rows
            .into_iter()
            .filter(|entry| {
                active
                    .get(&(entry.source.id, tenant.into()))
                    .is_some_and(|instance| instance.digest == entry.version.digest)
            })
            .flat_map(|entry| {
                let source = entry.source.id;
                entry
                    .version
                    .permissions
                    .into_iter()
                    .map(move |name| services::permission(source, &name))
            })
            .collect::<Vec<_>>();
        drop(active);
        if tenant == "development" {
            for (source, instance) in self.development.read().await.iter() {
                permissions.extend(
                    instance
                        .bundle
                        .manifest()
                        .plugin
                        .permissions
                        .iter()
                        .map(|name| services::permission(source, name)),
                );
            }
        }
        Ok(permissions)
    }
}
