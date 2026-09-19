use anyhow::{Context, Result, bail};
use kube::api::AttachParams;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::pvc;
use crate::runner_pod::RunnerPod;
use crate::util::{progress_bar, short_id};

const CHUNK: usize = 64 * 1024;

/// Streams a local tar file into `tar xf - -C /data` running in a pod that
/// mounts `pvc`, in 64KB chunks (same reasoning as backup: never blow up
/// memory on a large PVC). Target PVC must already exist — this tool never
/// creates one, see `pvc::ensure_exists`.
pub async fn run(
    client: kube::Client,
    namespace: &str,
    pvc_name: &str,
    in_path: &std::path::Path,
    uid: i64,
    gid: i64,
) -> Result<()> {
    pvc::ensure_exists(&client, namespace, pvc_name).await?;

    let pod_name = format!("pvc-migrator-restore-{}", short_id());
    tracing::info!("starting pod {pod_name} mounting PVC {pvc_name} in {namespace}");

    let runner = RunnerPod::create(client, namespace, &pod_name, pvc_name, uid, gid).await?;
    let result = do_restore(&runner, in_path).await;
    runner.delete().await;
    result
}

async fn do_restore(runner: &RunnerPod, in_path: &std::path::Path) -> Result<()> {
    let mut file = tokio::fs::File::open(in_path)
        .await
        .with_context(|| format!("opening local backup {}", in_path.display()))?;
    let total = file.metadata().await.ok().map(|m| m.len());
    let pb = progress_bar(total, "restore");

    let ap = AttachParams::default()
        .stdin(true)
        .stdout(true)
        .stderr(true);
    let mut process = runner
        .api()
        .exec(&runner.name, vec!["tar", "xf", "-", "-C", "/data"], &ap)
        .await
        .context("starting tar inside the pod")?;

    let mut transferred: u64 = 0;
    {
        let mut stdin = process.stdin().context("process has no stdin")?;
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = file.read(&mut buf).await.context("reading local backup")?;
            if n == 0 {
                break;
            }
            stdin
                .write_all(&buf[..n])
                .await
                .context("sending backup to tar")?;
            transferred += n as u64;
            pb.inc(n as u64);
        }
        // Explicit EOF: without this the remote tar keeps waiting for more
        // data forever, and the join() below never returns.
        stdin.shutdown().await.context("closing exec stdin")?;
    }
    pb.finish_with_message(format!("restore complete ({transferred} bytes)"));

    if let Err(err) = process.join().await {
        bail!("tar exited with error: {err:#}");
    }

    tracing::info!(
        "restore of {} ({transferred} bytes) completed on {}",
        in_path.display(),
        runner.name
    );
    Ok(())
}
