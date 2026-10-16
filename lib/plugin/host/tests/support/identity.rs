struct FixtureIdentityPlugin<P>(std::sync::Arc<P>);

impl<P> az_plugin_core::Plugin<az_plugin_host::runtime::server::plugins::RuntimeAssembly>
    for FixtureIdentityPlugin<P>
where
    P: az_plugin_host::identity::IdentityProvider
        + az_plugin_host::authorization::AuthorizationProvider,
{
    fn build<'a>(
        &'a self,
        assembly: &'a mut az_plugin_host::runtime::server::plugins::RuntimeAssembly,
    ) -> az_plugin_core::PluginFuture<'a> {
        Box::pin(async move {
            assembly
                .services
                .add_builder(self.0.clone())
                .bind::<dyn az_plugin_host::identity::IdentityProvider, P>()
                .bind::<dyn az_plugin_host::authorization::AuthorizationProvider, P>();
            Ok(())
        })
    }
}

pub fn with_identity<P>(
    identity: std::sync::Arc<P>,
    mut plugins: Vec<
        az_plugin_core::DynPlugin<az_plugin_host::runtime::server::plugins::RuntimeAssembly>,
    >,
) -> Vec<az_plugin_core::DynPlugin<az_plugin_host::runtime::server::plugins::RuntimeAssembly>>
where
    P: az_plugin_host::identity::IdentityProvider
        + az_plugin_host::authorization::AuthorizationProvider,
{
    plugins.push(std::sync::Arc::new(FixtureIdentityPlugin(identity)));
    plugins
}
