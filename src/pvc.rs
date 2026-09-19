use anyhow::{Context, Result, anyhow};
use k8s_openapi::api::core::v1::PersistentVolumeClaim;
use kube::Api;
use kube::api::AttachParams;
use tokio::io::AsyncReadExt;

use crate::runner_pod::RunnerPod;

/// Fails fast with a clear message instead of letting a missing PVC surface
/// later as an opaque "timed out waiting for pod Running" from the runner
/// pod failing to schedule.
pub async fn ensure_exists(client: &kube::Client, namespace: &str, name: &str) -> Result<()> {
    let api: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), namespace);
    api.get_opt(name)
        .await
        .context("querying PVC from the API")?
        .ok_or_else(|| {
            anyhow!(
                "PVC '{name}' does not exist in namespace '{namespace}'. On restore, it \
                 usually already comes from the app's own Helm release (even with \
                 replicaCount: 0) — check whether the Application has synced yet."
            )
        })?;
    Ok(())
}

/// Best-effort: `du -sk /data` inside the pod, converted to bytes. Returns
/// `None` if the command fails for any reason (busybox without `du`, an
/// empty PVC, whatever) — callers fall back to an indeterminate spinner
/// instead of letting a progress estimate block the backup.
pub async fn estimate_size_bytes(runner: &RunnerPod) -> Option<u64> {
    let ap = AttachParams::default().stdout(true).stderr(false);
    let mut process = runner
        .api()
        .exec(
            &runner.name,
            vec!["sh", "-c", "du -sk /data | cut -f1"],
            &ap,
        )
        .await
        .ok()?;

    let mut stdout = process.stdout()?;
    let mut out = String::new();
    stdout.read_to_string(&mut out).await.ok()?;
    let _ = process.join().await;

    out.trim().parse::<u64>().ok().map(|kb| kb * 1024)
}
