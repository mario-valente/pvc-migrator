# pvc-migrator

Copy the contents of a Kubernetes PVC to a local file (`backup`) or from a
local file into a PVC (`restore`) — a small Rust CLI built to move a
stateful app's data from one cluster to another (e.g. migrating off a
legacy cluster into a new one) without needing network connectivity between
the two clusters.

## How it works

No `kubectl` dependency, no direct cluster-to-cluster networking required.
The binary is the bridge:

1. It opens a Kubernetes API session to a cluster (via a kubeconfig +
   context), using [`kube-rs`](https://kube.rs) directly — no shelling out.
2. It creates a short-lived pod (`busybox`) that mounts the target PVC at
   `/data`.
3. It uses the Kubernetes `exec` API (the same protocol `kubectl exec` uses,
   over a WebSocket) to run `tar` inside that pod, and streams the tar
   stdout/stdin straight to/from a local file, in 64KB chunks — the whole
   PVC content is never buffered in memory, so this works the same for a
   10MB volume or a 300GB one.
4. The pod is deleted afterward (best-effort).

`backup` and `restore` are two completely independent commands, each
talking to its own cluster — run `backup` against the source cluster,
copy the resulting tar file however you like (scp, USB stick, whatever),
then run `restore` against the target cluster.

## Security

The throwaway pod is built to satisfy the Kubernetes `restricted` Pod
Security Standard: non-root, no privilege escalation, all capabilities
dropped, seccomp `RuntimeDefault`, read-only root filesystem (with an
`emptyDir` at `/tmp` for anything that insists on writing scratch files).
Without this it gets rejected outright by any namespace enforcing
`restricted` — which is common for anything touching real workload data.

**`--uid`/`--gid` matter.** The pod runs as whoever you tell it to (default
`1000`), also set as `fsGroup`. If the PVC's files are `0600` and owned by a
different UID (very common — e.g. the official Redis image writes as uid
`999`), the pod can't read them unless you pass the matching `--uid`. Find
the real owner first:

```bash
kubectl exec <pod-of-the-real-app> -- stat -c '%u:%g' /path/to/the/volume
```

## Usage

```bash
pvc-migrator backup \
  --kubeconfig ~/.kube/config --context <source-context> \
  --namespace <ns> --pvc <pvc-name> \
  --out backup.tar \
  --uid 999 --gid 999   # match the real data owner

pvc-migrator restore \
  --kubeconfig ~/.kube/config --context <target-context> \
  --namespace <ns> --pvc <pvc-name> \
  --in backup.tar \
  --uid 999 --gid 999
```

The target PVC must already exist (this tool never creates one) — in a
GitOps setup that's usually already true even for an app scaled to 0
replicas, since the PVC is applied by the Helm release independently of the
Deployment's replica count.

## Building / installing

Uses [`mise`](https://mise.jdx.dev) as the task runner:

```bash
mise run build     # cargo build --release
mise run install   # builds and installs into ~/.cargo/bin (cargo install --path .)
mise run test
mise run fmt
mise run clean
```

Without `mise`, the equivalent plain `cargo` commands work the same way
(`cargo build --release`, `cargo install --path .`, ...).

## Trying it out locally with KIND

`kind/redis.yaml` is a minimal Redis + PVC manifest used to exercise the
whole flow end to end with two local [KIND](https://kind.sigs.k8s.io/)
clusters, without touching anything real:

```bash
kind create cluster --name pvcmig-source
kind create cluster --name pvcmig-dest

kubectl --context kind-pvcmig-source apply -f kind/redis.yaml

# write some test data
POD=$(kubectl --context kind-pvcmig-source get pod -l app=redis -o jsonpath='{.items[0].metadata.name}')
kubectl --context kind-pvcmig-source exec "$POD" -- redis-cli SET hello world
kubectl --context kind-pvcmig-source exec "$POD" -- redis-cli SAVE

# back it up
mise run install
pvc-migrator backup --kubeconfig ~/.kube/config --context kind-pvcmig-source \
  --namespace default --pvc redis-data --out /tmp/redis.tar --uid 999 --gid 999

# create the PVC on the destination cluster (scaled to 0 so nothing mounts it yet)
kubectl --context kind-pvcmig-dest apply -f kind/redis.yaml
kubectl --context kind-pvcmig-dest scale deployment/redis --replicas=0
kubectl --context kind-pvcmig-dest wait --for=delete pod -l app=redis --timeout=60s

# restore it
pvc-migrator restore --kubeconfig ~/.kube/config --context kind-pvcmig-dest \
  --namespace default --pvc redis-data --in /tmp/redis.tar --uid 999 --gid 999

# bring it back up and check the data made it
kubectl --context kind-pvcmig-dest scale deployment/redis --replicas=1
kubectl --context kind-pvcmig-dest wait --for=condition=Ready pod -l app=redis --timeout=90s
POD=$(kubectl --context kind-pvcmig-dest get pod -l app=redis -o jsonpath='{.items[0].metadata.name}')
kubectl --context kind-pvcmig-dest exec "$POD" -- redis-cli GET hello
# -> world
```

## Known limitations

- No compression (`tar cf`, not `tar czf`) — busybox's gzip support isn't
  guaranteed across builds. Trade transfer size for portability; fine for
  local/same-datacenter transfers.
- Doesn't create the target PVC — it must already exist.
- One PVC per invocation. No resume, no retry, no checksum verification.
- Only tested against `ReadWriteOnce` volumes bound by a single-node
  provisioner (KIND's `local-path-provisioner`). Should work the same
  against any CSI driver since it's just a normal pod mount, but that's
  untested.
