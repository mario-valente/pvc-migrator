use anyhow::{Context, Result, bail};
use kube::api::AttachParams;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::pvc;
use crate::runner_pod::RunnerPod;
use crate::util::{progress_bar, short_id};

const CHUNK: usize = 64 * 1024;

/// Streams `tar cf - -C /data .` from a pod mounting `pvc` straight into a
/// local file, in 64KB chunks — never loads the whole PVC content into
/// memory, so it works the same for a 10MB Redis volume or a 300GB one. No
/// compression on purpose: keeps the busybox tar invocation portable (gzip
/// support varies across busybox builds) — trades transfer size for
/// simplicity, fine for local/same-datacenter transfers.
pub async fn run(
    client: kube::Client,
    namespace: &str,
    pvc_name: &str,
    out_path: &std::path::Path,
    uid: i64,
    gid: i64,
) -> Result<()> {
    pvc::ensure_exists(&client, namespace, pvc_name).await?;

    let pod_name = format!("pvc-migrator-backup-{}", short_id());
    tracing::info!("starting pod {pod_name} mounting PVC {pvc_name} in {namespace}");

    let runner = RunnerPod::create(client, namespace, &pod_name, pvc_name, uid, gid).await?;
    let result = do_backup(&runner, out_path).await;
    runner.delete().await;
    result
}

async fn do_backup(runner: &RunnerPod, out_path: &std::path::Path) -> Result<()> {
    let total = pvc::estimate_size_bytes(runner).await;
    let pb = progress_bar(total, "backup");

    let ap = AttachParams::default().stdout(true).stderr(true);
    let mut process = runner
        .api()
        .exec(&runner.name, vec!["tar", "cf", "-", "-C", "/data", "."], &ap)
        .await
        .context("starting tar inside the pod")?;

    let mut stdout = process.stdout().context("process has no stdout")?;
    let mut file = tokio::fs::File::create(out_path)
        .await
        .with_context(|| format!("creating local file {}", out_path.display()))?;

    let mut buf = vec![0u8; CHUNK];
    let mut transferred: u64 = 0;
    loop {
        let n = stdout.read(&mut buf).await.context("reading tar stdout")?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .await
            .context("writing local backup file")?;
        transferred += n as u64;
        pb.inc(n as u64);
    }
    file.flush().await?;
    pb.finish_with_message(format!("backup complete ({transferred} bytes)"));

    if let Err(err) = process.join().await {
        bail!("tar exited with error: {err:#}");
    }

    tracing::info!(
        "backup of {} saved to {} ({transferred} bytes)",
        runner.name,
        out_path.display()
    );
    Ok(())
}
