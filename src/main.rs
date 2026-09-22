mod backup;
mod k8s;
mod pvc;
mod restore;
mod runner_pod;
mod util;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// Copies the contents of a PVC to a local file (backup) or from a local
/// file into a PVC (restore), via a throwaway pod + `tar` — works between
/// any two clusters, even without direct network connectivity between them,
/// because this binary itself is the bridge (two separate API sessions).
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Packs a PVC's contents into a local tar file.
    Backup {
        #[arg(long)]
        kubeconfig: PathBuf,
        #[arg(long)]
        context: String,
        #[arg(long)]
        namespace: String,
        #[arg(long)]
        pvc: String,
        #[arg(long)]
        out: PathBuf,
        /// uid/gid the pod that mounts the PVC runs as (also set as
        /// fsGroup, to read files owned by anyone). Match it to the real
        /// owner of the data — e.g. 999 for official Redis, 1000 for Rails
        /// apps with `USER rails`, 0 for images that still run as root
        /// (e.g. NocoBase's official image). uid 0 drops the non-root
        /// requirement on the runner pod itself; it still won't get past a
        /// namespace that enforces `restricted` Pod Security.
        #[arg(long, default_value_t = 1000)]
        uid: i64,
        #[arg(long, default_value_t = 1000)]
        gid: i64,
    },
    /// Extracts a local tar file into a PVC (which must already exist).
    Restore {
        #[arg(long)]
        kubeconfig: PathBuf,
        #[arg(long)]
        context: String,
        #[arg(long)]
        namespace: String,
        #[arg(long)]
        pvc: String,
        #[arg(long = "in")]
        input: PathBuf,
        #[arg(long, default_value_t = 1000)]
        uid: i64,
        #[arg(long, default_value_t = 1000)]
        gid: i64,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "pvc_migrator=info".into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Backup {
            kubeconfig,
            context,
            namespace,
            pvc,
            out,
            uid,
            gid,
        } => {
            let client = k8s::client_for(&kubeconfig, &context).await?;
            backup::run(client, &namespace, &pvc, &out, uid, gid).await?;
        }
        Command::Restore {
            kubeconfig,
            context,
            namespace,
            pvc,
            input,
            uid,
            gid,
        } => {
            let client = k8s::client_for(&kubeconfig, &context).await?;
            restore::run(client, &namespace, &pvc, &input, uid, gid).await?;
        }
    }

    Ok(())
}
