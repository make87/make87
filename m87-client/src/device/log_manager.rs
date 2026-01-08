use anyhow::{Context, Result, anyhow};
use m87_shared::deploy_spec::{CommandSpec, LogSpec};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    sync::mpsc,
    time::timeout,
};
use tokio_util::sync::CancellationToken;

use crate::util::command::build_command;
use crate::util::format::format_log;

/// On-demand logs only:
/// - Snapshot: run once, capture bounded output (for `m87 <device> logs` and incident evidence)
/// - Follow: stream while a client is connected (`m87 <device> logs -f`)
///
/// No always-on processes.
#[derive(Clone)]
pub struct LogManager {
    tx: mpsc::Sender<LogCmd>,
}

enum LogCmd {
    Snapshot {
        unit_id: String,
        spec: LogSpec,
        env: BTreeMap<String, String>,
        workdir: PathBuf,
        max_bytes: usize,
        max_lines: usize,
        timeout: Duration,
        resp: mpsc::Sender<Result<Vec<String>>>,
    },
    FollowStart {
        unit_id: String,
        spec: LogSpec,
        env: BTreeMap<String, String>,
        workdir: PathBuf,
    },
    FollowStop {
        unit_id: String,
    },
    StopAll,
}

struct FollowStream {
    unit_id: String,
    cancel: CancellationToken,
    followers: u64,
}

impl LogManager {
    /// `event_tx` receives `TelemetryEvent::LogLine` only for active follow sessions.
    pub fn start() -> Self {
        let (tx, mut rx) = mpsc::channel::<LogCmd>(64);

        tokio::spawn(async move {
            let mut follows: HashMap<String, FollowStream> = HashMap::new();

            loop {
                match rx.recv().await {
                    Some(LogCmd::Snapshot {
                        unit_id,
                        spec,
                        env,
                        workdir,
                        max_bytes,
                        max_lines,
                        timeout: t,
                        resp,
                    }) => {
                        let r = snapshot_logs(
                            &unit_id, &spec.tail, &env, &workdir, max_bytes, max_lines, t,
                        )
                        .await;
                        let _ = resp.send(r).await;
                    }

                    Some(LogCmd::FollowStart {
                        unit_id,
                        spec,
                        env,
                        workdir,
                    }) => {
                        // Check if we already have a follow stream for this unit
                        if let Some(stream) = follows.get_mut(&unit_id) {
                            // Increment follower count
                            stream.followers += 1;
                            tracing::debug!(
                                "Added follower to {} (total: {})",
                                unit_id,
                                stream.followers
                            );
                            continue;
                        }

                        let Some(follow) = spec.follow.as_ref() else {
                            tracing::info!(
                                "Skipping follow for {} since there is no follow spec",
                                unit_id
                            );
                            continue;
                        };

                        match spawn_follow(&unit_id, &follow, &env, &workdir).await {
                            Ok((mut child)) => {
                                let uid = unit_id.clone();
                                let cancel = CancellationToken::new();
                                let cancel_clone = cancel.clone();

                                follows.insert(
                                    unit_id.clone(),
                                    FollowStream {
                                        unit_id: unit_id.clone(),
                                        cancel,
                                        followers: 1,
                                    },
                                );
                                tracing::debug!("Started follow stream for {}", unit_id);
                            }
                            Err(e) => {
                                tracing::error!("log follow spawn failed: {e}");
                            }
                        }
                    }

                    Some(LogCmd::FollowStop { unit_id }) => {
                        if let Some(stream) = follows.get_mut(&unit_id) {
                            stream.followers = stream.followers.saturating_sub(1);
                            tracing::debug!(
                                "Removed follower from {} (remaining: {})",
                                unit_id,
                                stream.followers
                            );

                            if stream.followers == 0 {
                                tracing::debug!(
                                    "No more followers for {}, cancelling stream",
                                    unit_id
                                );
                                let stream = follows.remove(&unit_id).unwrap();
                                stream.cancel.cancel();
                            }
                        }
                    }

                    Some(LogCmd::StopAll) => {
                        for (_, s) in follows.drain() {
                            s.cancel.cancel();
                        }
                    }

                    None => break,
                }
            }
        });

        Self { tx }
    }

    /// Run a bounded snapshot command once.
    /// Use for:
    /// - `m87 <device> logs` (non-follow)
    /// - incident evidence collection (last logs)
    pub async fn snapshot(
        &self,
        unit_id: String,
        spec: LogSpec,
        env: BTreeMap<String, String>,
        workdir: PathBuf,
        max_bytes: usize,
        max_lines: usize,
        timeout_dur: Duration,
    ) -> Result<Vec<String>> {
        let (resp_tx, mut resp_rx) = mpsc::channel::<Result<Vec<String>>>(1);
        let _ = self
            .tx
            .send(LogCmd::Snapshot {
                unit_id,
                spec,
                env,
                workdir,
                max_bytes,
                max_lines,
                timeout: timeout_dur,
                resp: resp_tx,
            })
            .await;

        resp_rx
            .recv()
            .await
            .ok_or_else(|| anyhow!("snapshot response channel closed"))?
    }

    /// Start streaming logs for a unit. Increments follower count.
    /// Multiple followers can watch the same unit; the stream is shared.
    pub async fn follow_start(
        &self,
        unit_id: String,
        spec: &LogSpec,
        env: BTreeMap<String, String>,
        workdir: PathBuf,
    ) {
        let _ = self
            .tx
            .send(LogCmd::FollowStart {
                unit_id,
                spec: spec.clone(),
                env,
                workdir,
            })
            .await;
    }

    /// Stop streaming logs for a unit. Decrements follower count.
    /// Stream is only cancelled when follower count reaches 0.
    pub async fn follow_stop(&self, unit_id: String) {
        let _ = self.tx.send(LogCmd::FollowStop { unit_id }).await;
    }

    pub async fn stop_all(&self) {
        let _ = self.tx.send(LogCmd::StopAll).await;
    }
}

async fn spawn_follow(
    unit_id: &str,
    spec: &CommandSpec,
    env: &BTreeMap<String, String>,
    workdir: &Path,
) -> Result<tokio::process::Child> {
    let mut cmd = build_command(spec)?;
    cmd.current_dir(workdir);
    for (k, v) in env {
        cmd.env(k, v);
    }

    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::null());

    let mut child = cmd.spawn().context("spawn command")?;
    let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
    let stderr = child.stderr.take().ok_or_else(|| anyhow!("no stderr"))?;

    let unit = unit_id.to_string();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::info!("{}", format_log(&unit, &line, true));
        }
    });
    let unit = unit_id.to_string();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::info!("{}", format_log(&unit, &line, true));
        }
    });

    Ok(child)
}

/// Runs the log command once and captures bounded output.
/// Assumes the provided command exits on its own (no -f).
async fn snapshot_logs(
    _unit_id: &str,
    spec: &CommandSpec,
    env: &BTreeMap<String, String>,
    workdir: &Path,
    max_bytes: usize,
    max_lines: usize,
    timeout_dur: Duration,
) -> Result<Vec<String>> {
    let mut cmd = build_command(spec)?;
    cmd.current_dir(workdir);
    for (k, v) in env {
        cmd.env(k, v);
    }
    // Capture stdout for snapshot
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::null());

    let mut child = cmd.spawn().context("spawn log snapshot command")?;
    let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
    let mut lines = BufReader::new(stdout).lines();

    let fut = async {
        let mut out = Vec::new();
        let mut bytes: usize = 0;

        while let Ok(Some(line)) = lines.next_line().await {
            bytes += line.len() + 1;
            out.push(line);

            if out.len() >= max_lines || bytes >= max_bytes {
                break;
            }
        }

        // Ensure process doesn't linger
        let _ = child.start_kill();
        let _ = child.wait().await;

        Ok::<_, anyhow::Error>(out)
    };

    timeout(timeout_dur, fut)
        .await
        .context("log snapshot timeout")?
}
