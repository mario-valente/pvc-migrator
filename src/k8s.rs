use anyhow::{Context, Result};
use kube::{
    Client,
    config::{KubeConfigOptions, Kubeconfig},
};
use std::path::Path;

/// Builds a Client for a given kubeconfig file + context name, so source and
/// target can be two entirely different clusters (even two different files).
pub async fn client_for(kubeconfig: &Path, context: &str) -> Result<Client> {
    let kubeconfig = Kubeconfig::read_from(kubeconfig)
        .with_context(|| format!("reading kubeconfig {}", kubeconfig.display()))?;

    let options = KubeConfigOptions {
        context: Some(context.to_string()),
        ..Default::default()
    };

    let config = kube::Config::from_custom_kubeconfig(kubeconfig, &options)
        .await
        .with_context(|| format!("resolving context '{context}' in kubeconfig"))?;

    Client::try_from(config).context("building client from config")
}
