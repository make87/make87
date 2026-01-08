use anyhow::{Context, Result, anyhow};
use m87_shared::deploy_spec::{
    CommandSpec, DeployReportKind, DeploymentRevision, DeploymentRevisionReport, HealthSpec,
    LivenessSpec, OnFailure, Outcome, RetrySpec, RollbackPolicy, RollbackReport, RunOnceKey,
    RunReport, RunSpec, RunState, RunType, Step, StepReport, UndoMode, WorkdirMode,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{fs, io::AsyncWriteExt, sync::RwLock, time::sleep};

use crate::{
    device::log_manager::LogManager,
    util::{
        command::{RunCommandError, build_command, run_command},
        shutdown::SHUTDOWN,
    },
};
const MAX_TAIL_BYTES: usize = 4 * 1024; // 4KB

fn data_dir() -> Result<PathBuf> {
    Ok(dirs::data_dir().context("data_dir")?.join("m87"))
}

fn events_dir() -> Result<PathBuf> {
    Ok(data_dir()?.join("events"))
}

fn pending_dir() -> Result<PathBuf> {
    Ok(events_dir()?.join("pending"))
}
fn inflight_dir() -> Result<PathBuf> {
    Ok(events_dir()?.join("inflight"))
}

fn unit_state_dir() -> Result<PathBuf> {
    Ok(data_dir()?.join("unit_state"))
}

async fn ensure_dirs() -> Result<()> {
    fs::create_dir_all(unit_state_dir()?).await?;
    fs::create_dir_all(pending_dir()?).await?;
    fs::create_dir_all(inflight_dir()?).await?;
    Ok(())
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct UnitLocalState {
    pub completed_run_once_keys: HashSet<String>,
    pub consecutive_health_failures: u32,
}

impl UnitLocalState {
    fn state_file_path(unit_id: &str) -> Result<PathBuf> {
        // Sanitize unit_id to make it filesystem-safe
        let safe_id = unit_id.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");

        let state_dir = unit_state_dir()?;
        Ok(state_dir.join(format!("{}.json", safe_id)))
    }

    fn load(unit_id: &str) -> Result<UnitLocalState> {
        let path = UnitLocalState::state_file_path(unit_id)?;

        if !path.exists() {
            return Ok(UnitLocalState::default());
        }

        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read state file for unit {}", unit_id))?;

        let state: UnitLocalState = serde_json::from_str(&contents)
            .with_context(|| format!("Failed to parse state file for unit {}", unit_id))?;

        Ok(state)
    }

    fn save(unit_id: &str, st: &UnitLocalState) -> Result<()> {
        let path = UnitLocalState::state_file_path(unit_id)?;

        let contents =
            serde_json::to_string_pretty(st).context("Failed to serialize unit state")?;

        std::fs::write(&path, contents)
            .with_context(|| format!("Failed to write state file for unit {}", unit_id))?;

        Ok(())
    }
}

pub struct RevisionStore {}

impl RevisionStore {
    fn desired_path() -> Result<PathBuf> {
        let config_dir = data_dir()?;
        let desired_path = config_dir.join("desired_units.json");
        Ok(desired_path)
    }

    fn previous_path() -> Result<PathBuf> {
        let config_dir = data_dir()?;
        let previous_path = config_dir.join("previous_units.json");
        Ok(previous_path)
    }

    // pub fn get_all() -> Result<HashMap<String, RunSpec>> {
    //     let desired_path = RevisionStore::desired_path()?;

    //     if !desired_path.exists() {
    //         return Ok(HashMap::new());
    //     }

    //     let contents =
    //         std::fs::read_to_string(&desired_path).context("Failed to read desired units file")?;
    //     let config: DeploymentRevision =
    //         serde_json::from_str(&contents).context("Failed to parse desired units file")?;

    //     Ok(config
    //         .units
    //         .iter()
    //         .map(|u| (u.get_id(), u.clone()))
    //         .collect())
    // }

    /// Get the current rollback policy
    pub fn get_rollback_policy() -> Result<Option<RollbackPolicy>> {
        let desired_path = RevisionStore::desired_path()?;
        if !desired_path.exists() {
            return Ok(None);
        }

        let contents =
            std::fs::read_to_string(&desired_path).context("Failed to read desired units file")?;
        let config: DeploymentRevision =
            serde_json::from_str(&contents).context("Failed to parse desired units file")?;

        Ok(config.rollback)
    }

    /// Get entire previous configuration for rollback
    pub fn get_previous_config() -> Result<Option<DeploymentRevision>> {
        let previous_path = RevisionStore::previous_path()?;
        if !previous_path.exists() {
            return Ok(None);
        }

        let contents = std::fs::read_to_string(&previous_path)
            .context("Failed to read previous units file")?;
        let config: DeploymentRevision =
            serde_json::from_str(&contents).context("Failed to parse previous units file")?;

        Ok(Some(config))
    }

    pub fn get_desired_config() -> Result<Option<DeploymentRevision>> {
        let desired_path = RevisionStore::desired_path()?;
        if !desired_path.exists() {
            return Ok(None);
        }

        let contents =
            std::fs::read_to_string(&desired_path).context("Failed to read desired units file")?;
        let config: DeploymentRevision =
            serde_json::from_str(&contents).context("Failed to parse desired units file")?;

        Ok(Some(config))
    }

    /// Set new desired configuration, backing up current to previous
    pub fn set_config(config: &DeploymentRevision) -> Result<()> {
        let previous_path = RevisionStore::previous_path()?;
        let desired_path = RevisionStore::desired_path()?;
        if desired_path.exists() {
            std::fs::copy(&desired_path, &previous_path)
                .context("Failed to backup previous units")?;
        }

        // Write new desired config
        let contents = serde_json::to_string_pretty(&config)
            .context("Failed to serialize desired units config")?;
        std::fs::write(&desired_path, contents).context("Failed to write desired units file")?;

        Ok(())
    }
}

#[derive(Clone)]
pub struct UnitManager {
    root_dir: PathBuf,
    dirty: Arc<RwLock<HashSet<String>>>,
    log_manager: LogManager,
    rollback_policy: Arc<RwLock<Option<RollbackPolicy>>>,
    deployment_started_at: Arc<RwLock<Option<Instant>>>,
}

impl UnitManager {
    /// Create a new UnitManager with a custom state store.
    pub async fn new() -> Result<Self> {
        let _ = ensure_dirs().await?;
        let _ = recover_inflight().await?;
        let root_dir = data_dir()?;

        let log_manager = LogManager::start();
        // Load rollback policy from disk if exists
        let rollback_policy = RevisionStore::get_rollback_policy().unwrap_or(None);

        Ok(Self {
            root_dir,
            dirty: Arc::new(RwLock::new(HashSet::new())),
            log_manager,
            rollback_policy: Arc::new(RwLock::new(rollback_policy)),
            deployment_started_at: Arc::new(RwLock::new(None)),
        })
    }

    /// Get reference to the log manager for external use (e.g., streams/logs routing)
    pub async fn start_log_follow(&self) -> Result<()> {
        if let Some(spec) = RevisionStore::get_desired_config()? {
            for (_, unit) in spec.get_unit_map() {
                if let Some(observer_spec) = &unit.observe {
                    if let Some(log_spec) = &observer_spec.logs {
                        let workdir = self.resolve_workdir(&unit).await?;
                        self.log_manager
                            .follow_start(unit.get_id(), log_spec, unit.env, workdir);
                    }
                }
            }
        }

        Ok(())
    }

    pub fn stop_log_follow(&self) -> Result<()> {
        if let Some(spec) = RevisionStore::get_desired_config()? {
            for (_, unit) in spec.get_unit_map() {
                self.log_manager.follow_stop(unit.get_id());
            }
        }
        Ok(())
    }

    /// Replace desired set (authoritative). Marks changes dirty.
    pub async fn set_desired_units(&self, config: DeploymentRevision) -> Result<()> {
        let mut new_map = HashMap::new();
        for u in &config.units {
            new_map.insert(u.get_id(), u.clone());
        }

        let old_desired = match RevisionStore::get_desired_config()? {
            Some(spec) => spec.get_unit_map(),
            None => BTreeMap::new(),
        };
        let mut dirty = self.dirty.write().await;

        // mark changed/added as dirty
        for (id, new_u) in &new_map {
            match old_desired.get(id) {
                Some(old_u) if unit_semantically_equal(old_u, new_u) => {}
                _ => {
                    dirty.insert(id.clone());
                }
            }
        }

        // mark removed as dirty so we can stop logs / stop service if needed
        for id in old_desired.keys() {
            if !new_map.contains_key(id) {
                dirty.insert(id.clone());
            }
        }

        // Update rollback policy cache
        *self.rollback_policy.write().await = config.rollback.clone();

        // Mark deployment start time for stabilization period
        *self.deployment_started_at.write().await = Some(Instant::now());

        RevisionStore::set_config(&config)?;
        Ok(())
    }

    /// Start the single supervisor loop.
    pub fn start(self: Arc<Self>) {
        tokio::spawn(async move {
            let mut next_health: HashMap<String, Instant> = HashMap::new();
            let mut next_liveness: HashMap<String, Instant> = HashMap::new();

            // coarse tick keeps CPU low; checks run only when due
            let tick = Duration::from_millis(250);

            loop {
                if SHUTDOWN.is_cancelled() {
                    break;
                }

                // 1) reconcile dirty changes (apply/stop/observe log streams)
                if let Err(e) = self.reconcile_dirty().await {
                    tracing::error!("reconcile error: {e}");
                    let Ok(Some(desired)) = RevisionStore::get_desired_config() else {
                        tracing::error!("no desired config found");
                        continue;
                    };

                    let _ = enqueue_event(DeployReportKind::DeploymentRevisionReport(
                        DeploymentRevisionReport {
                            revision_id: desired.get_id(),
                            outcome: Outcome::Failed,
                            dirty: true,
                            error: Some(format!("reconcile error: {e}")),
                        },
                    ))
                    .await;
                    // TODO: Rollback right away?
                }

                // 2) schedule/poll liveness + health only when due
                let now = Instant::now();
                let desired_spec = match RevisionStore::get_desired_config() {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!("failed to get all revisions: {e}");
                        // TODO: Rollback right away?
                        continue;
                    }
                };

                if let Some(spec) = desired_spec {
                    for (id, u) in spec.get_unit_map().iter() {
                        if !u.enabled {
                            continue;
                        }
                        let Some(obs) = &u.observe else {
                            continue;
                        };
                        let desired_revision_id = spec.get_id();

                        if let Some(liv) = &obs.liveness {
                            let due = next_liveness.get(id).copied().unwrap_or(now);
                            if now >= due {
                                next_liveness.insert(id.clone(), now + liv.every);
                                let _ = self
                                    .run_liveness_check(id, &desired_revision_id, u, liv)
                                    .await;
                            }
                        }
                        if let Some(health) = &obs.health {
                            let due = next_health.get(id).copied().unwrap_or(now);
                            if now >= due {
                                next_health.insert(id.clone(), now + health.every);
                                let _ = self
                                    .run_health_check(id, &desired_revision_id, u, health)
                                    .await;
                            }
                        }
                    }
                }

                sleep(tick).await;
            }
        });
    }

    async fn reconcile_dirty(&self) -> Result<()> {
        let dirty_ids: Vec<String> = {
            let dirty = self.dirty.read().await;
            if dirty.is_empty() {
                return Ok(());
            }
            dirty.iter().cloned().collect::<Vec<_>>()
        };

        let deploy_spec = RevisionStore::get_desired_config()?;
        let desired_snapshot = match &deploy_spec {
            Some(spec) => spec.units.iter().map(|u| (u.get_id(), u.clone())).collect(),
            None => HashMap::new(),
        };

        for id in dirty_ids {
            match desired_snapshot.get(&id) {
                None => {
                    // Unit removed - try to stop it using previous spec
                    if let Ok(Some(config)) = RevisionStore::get_previous_config() {
                        if let Some(prev_spec) = config.get_run(&id) {
                            if matches!(prev_spec.run_type, RunType::Service) {
                                let wd = match self.resolve_workdir(&prev_spec).await {
                                    Ok(wd) => wd,
                                    Err(_) => {
                                        // Can't resolve workdir, skip
                                        continue;
                                    }
                                };
                                let _ = self
                                    .stop_service(&prev_spec, &config.id.clone().unwrap(), &wd)
                                    .await;
                            }
                        }
                    }
                }
                Some(spec) => {
                    // Ensure workdir exists for service/job/observe (observe may still need a cwd)
                    let wd = self.resolve_workdir(spec).await?;

                    let desired_revision_id = match &deploy_spec {
                        Some(spec) => spec.get_id(),
                        None => {
                            tracing::warn!("No deploy spec provided for unit {:?}", spec);
                            continue;
                        }
                    };

                    // Apply/stop based on type
                    match spec.run_type {
                        RunType::Observe => {
                            // nothing else to execute
                        }
                        RunType::Job => {
                            if spec.enabled {
                                self.maybe_run_job(spec, &desired_revision_id, &wd).await?;
                            }
                        }
                        RunType::Service => {
                            if spec.enabled {
                                self.apply_service(spec, &desired_revision_id, &wd).await?;
                            } else {
                                self.stop_service(spec, &desired_revision_id, &wd).await?;
                            }
                        }
                    }
                }
            }
        }
        // Clear dirty set after processing
        self.dirty.write().await.clear();

        Ok(())
    }

    async fn maybe_run_job(&self, spec: &RunSpec, revision_id: &str, wd: &Path) -> Result<()> {
        // run_once gate
        if let Some(ro) = &spec.run_once {
            let key = match &ro.key {
                RunOnceKey::Auto(b) if !*b => None,
                RunOnceKey::Auto(_) => Some(spec.get_id()),
                RunOnceKey::Explicit(s) => Some(s.clone()),
            };

            if let Some(key) = key {
                let mut st = UnitLocalState::load(&spec.get_id())?;
                if st.completed_run_once_keys.contains(&key) {
                    // already done
                    return Ok(());
                }
                self.execute_unit_steps(spec, revision_id, wd).await?;
                st.completed_run_once_keys.insert(key);
                UnitLocalState::save(&spec.get_id(), &st)?;
                return Ok(());
            }
        }

        // normal job
        self.execute_unit_steps(spec, revision_id, wd).await
    }

    async fn apply_service(&self, spec: &RunSpec, revision_id: &str, wd: &Path) -> Result<()> {
        self.execute_unit_steps(spec, revision_id, wd).await
    }

    async fn stop_service(&self, spec: &RunSpec, revision_id: &str, wd: &Path) -> Result<()> {
        if let Some(stop) = &spec.stop {
            self.execute_steps(
                &spec.get_id(),
                revision_id,
                wd,
                &spec.env,
                &stop.steps,
                spec.on_failure.as_ref(),
            )
            .await
        } else {
            Ok(())
        }
    }

    async fn execute_unit_steps(&self, spec: &RunSpec, revision_id: &str, wd: &Path) -> Result<()> {
        // materialize files (only if any)
        self.materialize_files(spec, wd).await?;

        match self
            .execute_steps(
                &spec.get_id(),
                &revision_id.to_string(),
                wd,
                &spec.env,
                &spec.steps,
                spec.on_failure.as_ref(),
            )
            .await
        {
            Ok(()) => {
                let _ = enqueue_event(DeployReportKind::RunReport(RunReport {
                    run_id: spec.get_id(),
                    revision_id: revision_id.to_string(),
                    outcome: Outcome::Success,
                    error: None,
                }))
                .await;
                Ok(())
            }
            Err(e) => {
                let _ = enqueue_event(DeployReportKind::RunReport(RunReport {
                    run_id: spec.get_id(),
                    revision_id: revision_id.to_string(),
                    outcome: Outcome::Failed,
                    error: Some(e.to_string()),
                }))
                .await;
                Err(e)
            }
        }
    }

    async fn run_liveness_check(
        &self,
        unit_id: &str,
        revision_id: &str,
        spec: &RunSpec,
        liv: &LivenessSpec,
    ) -> Result<()> {
        let wd = self.resolve_workdir(spec).await?;
        let res = run_command(
            unit_id,
            &wd,
            &spec.env,
            &liv.check,
            Some(Duration::from_secs(30)),
            64, // we dont need to tail the output
        )
        .await;
        match res {
            Ok(_) => {
                let _ = enqueue_event(DeployReportKind::RunState(RunState {
                    run_id: unit_id.to_string(),
                    revision_id: revision_id.to_string(),
                    healthy: None,
                    alive: Some(true),
                    report_time: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64,
                }))
                .await;
                Ok(())
            }
            Err(e) => {
                let _ = enqueue_event(DeployReportKind::RunState(RunState {
                    run_id: unit_id.to_string(),
                    revision_id: revision_id.to_string(),
                    healthy: Some(false),
                    alive: Some(false),
                    report_time: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64,
                }))
                .await;

                // Check if we should trigger rollback
                let _ = self.check_rollback_on_liveness_failure(unit_id).await;

                Err(e.into())
            }
        }
    }

    async fn run_health_check(
        &self,
        unit_id: &str,
        revision_id: &str,
        spec: &RunSpec,
        health: &HealthSpec,
    ) -> Result<()> {
        let wd = self.resolve_workdir(spec).await?;
        let mut st = UnitLocalState::load(unit_id)?;

        let res = run_command(
            unit_id,
            &wd,
            &spec.env,
            &health.run,
            Some(Duration::from_secs(30)),
            64, // we dont need to tail the output
        )
        .await;
        match res {
            Ok(_tail) => {
                st.consecutive_health_failures = 0;
                UnitLocalState::save(unit_id, &st)?;
                let _ = enqueue_event(DeployReportKind::RunState(RunState {
                    run_id: unit_id.to_string(),
                    revision_id: revision_id.to_string(),
                    healthy: Some(true),
                    alive: Some(true),
                    report_time: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64,
                }))
                .await;
                Ok(())
            }
            Err(e) => {
                st.consecutive_health_failures = st.consecutive_health_failures.saturating_add(1);
                let consecutive = st.consecutive_health_failures;
                UnitLocalState::save(unit_id, &st)?;

                let _ = enqueue_event(DeployReportKind::RunState(RunState {
                    run_id: unit_id.to_string(),
                    revision_id: revision_id.to_string(),
                    healthy: Some(false),
                    alive: None,
                    report_time: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64,
                }))
                .await;
                // Check if we should trigger rollback
                let _ = self
                    .check_rollback_on_health_failure(revision_id, consecutive)
                    .await;

                // note: incident engine decides what to do when consecutive >= fails_after
                if consecutive >= health.fails_after {
                    // keep returning Err; caller ignores
                    let _ = enqueue_event(DeployReportKind::RunState(RunState {
                        run_id: unit_id.to_string(),
                        revision_id: revision_id.to_string(),
                        healthy: Some(false),
                        alive: None,
                        report_time: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_millis() as u64,
                    }))
                    .await;
                    return Err(e.into());
                }
                Ok(())
            }
        }
    }

    async fn check_rollback_on_health_failure(
        &self,
        revision_id: &str,
        consecutive: u32,
    ) -> Result<()> {
        use m87_shared::deploy_spec::RollbackTrigger;

        let policy = match &*self.rollback_policy.read().await {
            Some(p) => p.clone(),
            None => return Ok(()), // No rollback policy configured
        };

        // Check if we're still in stabilization period
        if !self.is_past_stabilization_period(&policy).await {
            return Ok(()); // Don't rollback during stabilization
        }

        let should_rollback = match &policy.on_health_failure {
            RollbackTrigger::Never => false,
            RollbackTrigger::Any => consecutive > 0,
            RollbackTrigger::All => {
                // Check if ALL units have health failures
                self.check_all_units_failing().await?
            }
            RollbackTrigger::Consecutive(n) => consecutive >= *n,
        };

        if should_rollback {
            tracing::warn!(
                "Health failure triggered rollback for revision_id {}",
                revision_id
            );
            self.trigger_rollback(revision_id).await?;
        }

        Ok(())
    }

    async fn check_rollback_on_liveness_failure(&self, revision_id: &str) -> Result<()> {
        use m87_shared::deploy_spec::RollbackTrigger;

        let policy = match &*self.rollback_policy.read().await {
            Some(p) => p.clone(),
            None => return Ok(()), // No rollback policy configured
        };

        // Check if we're still in stabilization period
        if !self.is_past_stabilization_period(&policy).await {
            return Ok(()); // Don't rollback during stabilization
        }

        let should_rollback = match &policy.on_liveness_failure {
            RollbackTrigger::Never => false,
            RollbackTrigger::Any => true,
            RollbackTrigger::All => {
                // Check if ALL units have liveness failures
                self.check_all_units_failing().await?
            }
            RollbackTrigger::Consecutive(_) => {
                // Liveness doesn't track consecutive failures, treat as "Any"
                true
            }
        };

        if should_rollback {
            tracing::warn!(
                "Liveness failure triggered rollback for revision_id {}",
                revision_id
            );
            self.trigger_rollback(revision_id).await?;
        }

        Ok(())
    }

    async fn is_past_stabilization_period(&self, policy: &RollbackPolicy) -> bool {
        let deployment_time = self.deployment_started_at.read().await;

        match *deployment_time {
            None => true, // No deployment time tracked, allow rollback
            Some(start) => {
                let elapsed = start.elapsed();
                elapsed.as_secs() >= policy.stabilization_period_secs
            }
        }
    }

    async fn check_all_units_failing(&self) -> Result<bool> {
        let desired = match RevisionStore::get_desired_config()? {
            Some(config) => config.get_unit_map(),
            None => return Ok(false),
        };

        if desired.is_empty() {
            return Ok(false);
        }

        let mut all_failing = true;
        for (id, _spec) in &desired {
            if let Ok(st) = UnitLocalState::load(id) {
                if st.consecutive_health_failures == 0 {
                    all_failing = false;
                    break;
                }
            }
        }

        Ok(all_failing)
    }

    async fn trigger_rollback(&self, revision_id: &str) -> Result<()> {
        tracing::warn!("ROLLBACK TRIGGERED - Reverting to previous configuration");

        // Load previous configuration
        let prev_config = match RevisionStore::get_previous_config()? {
            Some(config) => config,
            None => {
                tracing::error!("No previous configuration available for rollback");
                let _ = enqueue_event(DeployReportKind::RollbackReport(RollbackReport {
                    revision_id: revision_id.to_string(),
                    success: false,
                    undone_steps: vec![],
                    error: Some("No previous configuration available".to_string()),
                    log_tail: "".to_string(),
                }));
                return Err(anyhow!("No previous configuration available"));
            }
        };

        tracing::info!(
            "Rolling back to previous configuration with {} units",
            prev_config.units.len()
        );

        // Apply previous configuration (this will reset deployment_started_at)
        self.set_desired_units(prev_config).await?;

        // TODO: this jsut changes the target revision. Rollback happens in the main loop ehwn this returns
        let _ = enqueue_event(DeployReportKind::RollbackReport(RollbackReport {
            revision_id: revision_id.to_string(),
            success: true,
            undone_steps: vec![],
            error: None,
            log_tail: "".to_string(),
        }));

        tracing::info!("Rollback complete");
        Ok(())
    }

    async fn execute_steps(
        &self,
        unit_id: &str,
        revision_id: &str,
        wd: &Path,
        env: &BTreeMap<String, String>,
        steps: &[Step],
        on_failure: Option<&OnFailure>,
    ) -> Result<()> {
        let policy = on_failure.cloned().unwrap_or(OnFailure {
            undo: UndoMode::None,
            continue_on_failure: false,
        });

        let mut executed: Vec<&Step> = Vec::new();

        for step in steps {
            let res = self
                .run_step_with_retry(unit_id, revision_id, wd, env, step)
                .await;
            match res {
                Ok(()) => executed.push(step),
                Err(e) => {
                    // Undo
                    match policy.undo {
                        UndoMode::None => {}
                        UndoMode::ExecutedSteps => {
                            self.undo_steps(unit_id, revision_id, wd, env, &executed)
                                .await;
                        }
                    }

                    if policy.continue_on_failure {
                        continue;
                    }
                    return Err(e);
                }
            }
        }

        Ok(())
    }

    async fn undo_steps(
        &self,
        unit_id: &str,
        revision_id: &str,
        wd: &Path,
        env: &BTreeMap<String, String>,
        steps: &[&Step],
    ) {
        for step in steps.iter().rev() {
            if let Some(undo) = &step.undo {
                // if undo fails we dont care for now. run_step takes care of sending the event to the user
                let _ = run_step(unit_id, wd, env, step, revision_id, 0, MAX_TAIL_BYTES).await;
            }
        }
    }

    async fn run_step_with_retry(
        &self,
        unit_id: &str,
        revision_id: &str,
        wd: &Path,
        env: &BTreeMap<String, String>,
        step: &Step,
    ) -> Result<()> {
        let retry = step.retry.clone().unwrap_or(RetrySpec {
            attempts: 1,
            backoff: Duration::from_millis(0),
            on_exit_codes: None,
        });

        let attempts = retry.attempts.max(1);
        for i in 0..attempts {
            let res = run_step(unit_id, wd, env, step, revision_id, i, MAX_TAIL_BYTES).await;
            match res {
                Ok(report) => return Ok(()),
                Err(e) => {
                    if i + 1 >= attempts {
                        return Err(e);
                    }
                    sleep(retry.backoff).await;
                }
            }
        }

        return Err(anyhow!("Failed to run command"));
    }

    async fn resolve_workdir(&self, spec: &RunSpec) -> Result<PathBuf> {
        // observe-only still gets a deterministic cwd for relative paths in log/health commands
        let base = if let Some(wd) = &spec.workdir {
            if let Some(p) = &wd.path {
                PathBuf::from(p)
            } else {
                self.root_dir.join("units").join(&spec.get_id())
            }
        } else {
            self.root_dir.join("units").join(&spec.get_id())
        };

        // choose persistent/ephemeral
        let mode = spec
            .workdir
            .as_ref()
            .map(|w| w.mode.clone())
            .unwrap_or(WorkdirMode::Persistent);

        let resolved = match mode {
            WorkdirMode::Persistent => base,
            WorkdirMode::Ephemeral => self.root_dir.join("tmp").join("units").join(&spec.get_id()),
        };

        tokio::fs::create_dir_all(&resolved).await?;
        Ok(resolved)
    }

    async fn materialize_files(&self, spec: &RunSpec, wd: &Path) -> Result<()> {
        if spec.files.is_empty() {
            return Ok(());
        }
        for (rel, content) in &spec.files {
            let p = wd.join(rel);
            if let Some(parent) = p.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::write(&p, content).await?;
        }
        Ok(())
    }
}

fn unit_semantically_equal(a: &RunSpec, b: &RunSpec) -> bool {
    a.get_id() == b.get_id()
}

pub async fn enqueue_event(event: DeployReportKind) -> Result<()> {
    ensure_dirs().await?;

    let id = format!(
        "{}-{}",
        chrono::Utc::now().timestamp_millis(),
        rand_suffix()
    );
    let pending = pending_dir()?.join(format!("{id}.json"));
    let tmp = pending.with_extension("json.tmp");

    let bytes = serde_json::to_vec(&event).context("serialize event")?;

    let mut f = fs::File::create(&tmp).await.context("create tmp")?;
    f.write_all(&bytes).await.context("write tmp")?;
    f.flush().await.context("flush tmp")?;
    drop(f);

    fs::rename(&tmp, &pending)
        .await
        .context("atomic rename tmp->pending")?;
    Ok(())
}

fn rand_suffix() -> u32 {
    // no extra crate needed; weak but fine for 1/day
    (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos()) as u32
}

pub struct ClaimedEvent {
    pub path: PathBuf, // inflight file path
    pub report: DeployReportKind,
}

pub async fn recover_inflight() -> Result<()> {
    ensure_dirs().await?;
    let inflight = inflight_dir()?;
    let pending = pending_dir()?;

    let mut rd = fs::read_dir(&inflight).await?;
    while let Some(e) = rd.next_entry().await? {
        let p = e.path();
        if p.extension().and_then(|s| s.to_str()) == Some("json") {
            let target = pending.join(p.file_name().unwrap());
            let _ = fs::rename(&p, &target).await;
        }
    }
    Ok(())
}

pub async fn claim_next_event() -> Result<Option<ClaimedEvent>> {
    ensure_dirs().await?;

    let pending = pending_dir()?;
    let inflight = inflight_dir()?;

    // List pending files; pick oldest by filename (timestamp prefix)
    let mut files = Vec::new();
    let mut rd = fs::read_dir(&pending).await?;
    while let Some(e) = rd.next_entry().await? {
        let p = e.path();
        if p.extension().and_then(|s| s.to_str()) == Some("json") {
            files.push(p);
        }
    }
    files.sort(); // works if filename starts with timestamp

    let Some(p) = files.first().cloned() else {
        return Ok(None);
    };

    let inflight_path = inflight.join(p.file_name().unwrap());
    fs::rename(&p, &inflight_path)
        .await
        .context("claim rename pending->inflight")?;

    let bytes = fs::read(&inflight_path).await.context("read inflight")?;
    let event: DeployReportKind = serde_json::from_slice(&bytes).context("parse inflight")?;

    Ok(Some(ClaimedEvent {
        path: inflight_path,
        report: event,
    }))
}

pub async fn ack_event(claimed: &ClaimedEvent) -> Result<()> {
    fs::remove_file(&claimed.path)
        .await
        .context("delete inflight")?;
    Ok(())
}

pub async fn on_new_event() -> Option<ClaimedEvent> {
    loop {
        // Try immediately (covers backlog + missed cycles)
        match claim_next_event().await {
            Ok(Some(ev)) => return Some(ev),
            Ok(None) => {
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
            Err(e) => {
                tracing::error!("event queue error: {e}");
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        }
    }
}

async fn run_step(
    unit_id: &str,
    wd: &Path,
    env: &BTreeMap<String, String>,
    step: &Step,
    revision_id: &str,
    i: u32,
    max_tail_bytes: usize,
) -> Result<()> {
    let res = run_command(unit_id, wd, env, &step.run, step.timeout, max_tail_bytes).await;
    let res = match res {
        Ok(tail) => Ok(StepReport {
            revision_id: revision_id.to_string(),
            run_id: unit_id.to_string(),
            name: step.name.clone(),
            attempts: i + 1,
            log_tail: tail,
            exit_code: None,
            success: true,
            error: None,
            report_time: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        }),
        Err(RunCommandError::Other(e)) => Err(e),
        Err(RunCommandError::Io(e)) => Err(e.into()),
        Err(RunCommandError::Failed(e)) => Ok(StepReport {
            revision_id: revision_id.to_string(),
            run_id: unit_id.to_string(),
            name: step.name.clone(),
            attempts: i + 1,
            log_tail: e.combined_tail,
            exit_code: e.exit_code,
            success: false,
            error: e.error,
            report_time: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        }),
    };
    match res {
        Ok(report) => {
            enqueue_event(DeployReportKind::StepReport(report.clone())).await?;
            if !report.success {
                Err(anyhow!(
                    "Step {} failed: {}",
                    step.name.clone().unwrap_or("unknown step".to_string()),
                    report.error.unwrap_or("unknown error".to_string())
                ))
            } else {
                Ok(())
            }
        }

        Err(e) => Err(e),
    }
}
