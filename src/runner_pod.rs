use anyhow::{Context, Result, bail};
use k8s_openapi::api::core::v1::{
    Capabilities, Container, EmptyDirVolumeSource, PersistentVolumeClaimVolumeSource, Pod,
    PodSecurityContext, PodSpec, SeccompProfile, SecurityContext, Volume, VolumeMount,
};
use kube::{
    Api,
    api::{DeleteParams, PostParams, WatchEvent, WatchParams},
};
use futures::StreamExt;
use std::time::Duration;

/// Name/image of the throwaway pod used to mount a PVC and stream its
/// contents through `tar`. Busybox keeps the image small and always has
/// `tar`/`sh`/`du` — no compression flags used, to dodge gzip-support
/// variance across busybox builds.
const RUNNER_IMAGE: &str = "busybox:1.36";

pub struct RunnerPod {
    pub name: String,
    api: Api<Pod>,
}

impl RunnerPod {
    /// Creates a pod in `namespace` mounting `pvc_name` at /data, running as
    /// `uid`/`gid` (also set as `fsGroup`, so the mounted volume's files are
    /// group-readable/writable regardless of who originally wrote them —
    /// same trick most Helm charts use to let a non-root container touch a
    /// PV it doesn't own). Security context is built to satisfy Pod Security
    /// `restricted`: non-root, no privilege escalation, all capabilities
    /// dropped, seccomp RuntimeDefault, read-only root filesystem (an
    /// `emptyDir` covers /tmp for anything that insists on scratch space).
    /// Without this, the pod is rejected outright by any `restricted`
    /// namespace.
    pub async fn create(
        client: kube::Client,
        namespace: &str,
        name: &str,
        pvc_name: &str,
        uid: i64,
        gid: i64,
    ) -> Result<Self> {
        let api: Api<Pod> = Api::namespaced(client, namespace);

        let pod = Pod {
            metadata: kube::api::ObjectMeta {
                name: Some(name.to_string()),
                labels: Some(
                    [("app".to_string(), "pvc-migrator".to_string())]
                        .into_iter()
                        .collect(),
                ),
                ..Default::default()
            },
            spec: Some(PodSpec {
                restart_policy: Some("Never".to_string()),
                security_context: Some(PodSecurityContext {
                    // Some PVCs are genuinely owned by root (legacy apps that
                    // never dropped privileges — e.g. the official NocoBase
                    // image). Forcing runAsNonRoot: true here would make
                    // Kubernetes reject the pod outright whenever --uid 0 is
                    // passed on purpose. Only claim non-root when uid != 0;
                    // a namespace that actually enforces `restricted` PSA
                    // will still (correctly) refuse a uid-0 pod on its own.
                    run_as_non_root: Some(uid != 0),
                    run_as_user: Some(uid),
                    run_as_group: Some(gid),
                    fs_group: Some(gid),
                    seccomp_profile: Some(SeccompProfile {
                        type_: "RuntimeDefault".to_string(),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                containers: vec![Container {
                    name: "runner".to_string(),
                    image: Some(RUNNER_IMAGE.to_string()),
                    command: Some(vec!["sleep".to_string(), "3600".to_string()]),
                    security_context: Some(SecurityContext {
                        allow_privilege_escalation: Some(false),
                        read_only_root_filesystem: Some(true),
                        run_as_non_root: Some(uid != 0),
                        capabilities: Some(Capabilities {
                            drop: Some(vec!["ALL".to_string()]),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    volume_mounts: Some(vec![
                        VolumeMount {
                            name: "data".to_string(),
                            mount_path: "/data".to_string(),
                            ..Default::default()
                        },
                        VolumeMount {
                            name: "tmp".to_string(),
                            mount_path: "/tmp".to_string(),
                            ..Default::default()
                        },
                    ]),
                    ..Default::default()
                }],
                volumes: Some(vec![
                    Volume {
                        name: "data".to_string(),
                        persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                            claim_name: pvc_name.to_string(),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    Volume {
                        name: "tmp".to_string(),
                        empty_dir: Some(EmptyDirVolumeSource::default()),
                        ..Default::default()
                    },
                ]),
                ..Default::default()
            }),
            ..Default::default()
        };

        api.create(&PostParams::default(), &pod)
            .await
            .with_context(|| format!("creating pod {name} in {namespace}"))?;

        wait_running(&api, name).await?;

        Ok(Self {
            name: name.to_string(),
            api,
        })
    }

    pub fn api(&self) -> &Api<Pod> {
        &self.api
    }

    /// Best-effort delete; failures are logged, not propagated, so cleanup
    /// never masks the real error from backup/restore.
    pub async fn delete(&self) {
        if let Err(err) = self.api.delete(&self.name, &DeleteParams::default()).await {
            tracing::warn!("failed to delete pod {}: {err:#}", self.name);
        }
    }
}

async fn wait_running(api: &Api<Pod>, name: &str) -> Result<()> {
    let wp = WatchParams::default()
        .fields(&format!("metadata.name={name}"))
        .timeout(60);

    let mut stream = api.watch(&wp, "0").await?.boxed();

    let deadline = tokio::time::sleep(Duration::from_secs(90));
    tokio::pin!(deadline);

    loop {
        tokio::select! {
            _ = &mut deadline => bail!("timed out waiting for pod {name} to become Running"),
            ev = stream.next() => {
                let Some(ev) = ev else { bail!("watch on {name} ended before it became Running") };
                if let WatchEvent::Modified(pod) | WatchEvent::Added(pod) = ev? {
                    if let Some(status) = &pod.status {
                        if status.phase.as_deref() == Some("Running") {
                            return Ok(());
                        }
                        if status.phase.as_deref() == Some("Failed") {
                            bail!("pod {name} entered Failed before becoming ready");
                        }
                    }
                }
            }
        }
    }
}
