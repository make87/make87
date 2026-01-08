use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

fn sha256_hex(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}

fn hash_json<T: Serialize>(v: &T) -> String {
    let data = serde_json::to_vec(v).expect("hash_json serialization must not fail");
    sha256_hex(data)
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentRevision {
    // sha2 hash of the deployment revision
    #[serde(default)]
    pub id: Option<String>,
    pub units: Vec<RunSpec>,
    #[serde(default)]
    pub rollback: Option<RollbackPolicy>,
}

impl DeploymentRevision {
    pub fn new(units: Vec<RunSpec>, rollback: Option<RollbackPolicy>) -> Self {
        let mut rev = Self {
            id: None,
            units,
            rollback,
        };
        rev.ensure_ids();
        rev
    }

    pub fn get_id(&self) -> String {
        match &self.id {
            Some(id) => id.clone(),
            None => {
                let mut spec = self.clone();
                spec.ensure_ids();
                spec.id.unwrap()
            }
        }
    }

    pub fn get_unit_map(&self) -> BTreeMap<String, RunSpec> {
        self.units.iter().map(|u| (u.get_id(), u.clone())).collect()
    }

    pub fn get_run(&self, run_id: &str) -> Option<RunSpec> {
        let res = self.units.iter().find(|u| u.get_id() == run_id);
        res.cloned()
    }

    pub fn ensure_ids(&mut self) {
        // 1) ensure children ids exist
        for u in &mut self.units {
            u.ensure_id();
        }
        if let Some(r) = &mut self.rollback {
            r.ensure_id();
        }

        // 2) compute revision id only from child ids
        if self.id.is_none() {
            let mut hasher = Sha256::new();
            for u in &self.units {
                hasher.update(u.id.as_deref().expect("ensure_id set").as_bytes());
            }
            if let Some(r) = &self.rollback {
                hasher.update(r.id.as_deref().expect("ensure_id set").as_bytes());
            }
            self.id = Some(format!("{:x}", hasher.finalize()));
        }
    }

    pub fn from_yaml(yaml: &str) -> Result<Self, serde_yaml::Error> {
        let mut rev: Self = serde_yaml::from_str(yaml)?;
        rev.ensure_ids();
        Ok(rev)
    }

    pub fn to_yaml(&self) -> Result<String, serde_yaml::Error> {
        serde_yaml::to_string(self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackPolicy {
    // sha2 hash of the rollback policy
    #[serde(default)]
    pub id: Option<String>,
    /// Automatically rollback if health checks fail
    #[serde(default)]
    pub on_health_failure: RollbackTrigger,
    /// Automatically rollback if liveness checks fail
    #[serde(default)]
    pub on_liveness_failure: RollbackTrigger,
    /// Time window to monitor for failures before considering deployment stable
    #[serde(default = "default_stabilization_period")]
    pub stabilization_period_secs: u64,
}

impl RollbackPolicy {
    pub fn new(
        on_health_failure: RollbackTrigger,
        on_liveness_failure: RollbackTrigger,
        stabilization_period_secs: u64,
    ) -> Self {
        let mut p = Self {
            id: None,
            on_health_failure,
            on_liveness_failure,
            stabilization_period_secs,
        };
        p.ensure_id();
        p
    }

    pub fn ensure_id(&mut self) {
        if self.id.is_none() {
            // hash policy content (excluding id)
            let data = serde_json::to_vec(&(
                &self.on_health_failure,
                &self.on_liveness_failure,
                self.stabilization_period_secs,
            ))
            .expect("serialize");
            self.id = Some(format!("{:x}", Sha256::digest(data)));
        }
    }
}

fn default_stabilization_period() -> u64 {
    60 // 1 minute
}

#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RollbackTrigger {
    /// Never rollback automatically
    #[default]
    Never,
    /// Rollback if any unit fails
    Any,
    /// Rollback only if all units fail
    All,
    /// Rollback if a specific number of consecutive failures
    Consecutive(u32),
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct RunSpec {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub run_type: RunType,
    pub enabled: bool,

    // service / job only
    #[serde(default)]
    pub workdir: Option<Workdir>,
    #[serde(default)]
    pub files: BTreeMap<String, String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub run_once: Option<RunOnce>,

    #[serde(default)]
    pub steps: Vec<Step>,
    #[serde(default)]
    pub on_failure: Option<OnFailure>,
    #[serde(default)]
    pub stop: Option<StopSpec>,
    #[serde(default)]
    pub reboot: RebootMode,

    #[serde(default)]
    pub observe: Option<ObserveSpec>,
}

impl RunSpec {
    pub fn new(
        run_type: RunType,
        enabled: bool,
        workdir: Option<Workdir>,
        files: BTreeMap<String, String>,
        env: BTreeMap<String, String>,
        run_once: Option<RunOnce>,
        steps: Vec<Step>,
        on_failure: Option<OnFailure>,
        stop: Option<StopSpec>,
        reboot: RebootMode,
        observe: Option<ObserveSpec>,
    ) -> Self {
        let mut s = Self {
            id: None,
            run_type,
            enabled,
            workdir,
            files,
            env,
            run_once,
            steps,
            on_failure,
            stop,
            reboot,
            observe,
        };
        s.ensure_id();
        s
    }

    pub fn get_id(&self) -> String {
        match &self.id {
            Some(id) => id.clone(),
            None => {
                let mut spec = self.clone();
                spec.ensure_id();
                spec.id.unwrap()
            }
        }
    }

    pub fn ensure_id(&mut self) {
        if self.id.is_none() {
            // You can decide what constitutes identity.
            // This hashes everything except id itself.
            #[derive(Serialize)]
            struct RunSpecHashView<'a> {
                run_type: &'a RunType,
                enabled: bool,
                workdir: &'a Option<Workdir>,
                files: &'a BTreeMap<String, String>,
                env: &'a BTreeMap<String, String>,
                run_once: &'a Option<RunOnce>,
                steps: &'a [Step],
                on_failure: &'a Option<OnFailure>,
                stop: &'a Option<StopSpec>,
                reboot: &'a RebootMode,
                observe: &'a Option<ObserveSpec>,
            }

            let view = RunSpecHashView {
                run_type: &self.run_type,
                enabled: self.enabled,
                workdir: &self.workdir,
                files: &self.files,
                env: &self.env,
                run_once: &self.run_once,
                steps: &self.steps,
                on_failure: &self.on_failure,
                stop: &self.stop,
                reboot: &self.reboot,
                observe: &self.observe,
            };

            self.id = Some(hash_json(&view));
        }
    }

    pub fn from_yaml(yaml: &str) -> Result<Self, serde_yaml::Error> {
        let mut rev: Self = serde_yaml::from_str(yaml)?;
        rev.ensure_id();
        Ok(rev)
    }
}

#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum RunType {
    #[default]
    Service,
    Job,
    Observe,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum RebootMode {
    #[default]
    None,
    Request,
    Auto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    #[serde(default)]
    pub name: Option<String>,
    pub run: CommandSpec,
    #[serde(default)]
    pub timeout: Option<Duration>,
    #[serde(default)]
    pub retry: Option<RetrySpec>,
    #[serde(default)]
    pub undo: Option<Undo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Undo {
    pub run: CommandSpec,
    #[serde(default)]
    pub timeout: Option<Duration>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OnFailure {
    #[serde(default)]
    pub undo: UndoMode,
    #[serde(default)]
    pub continue_on_failure: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UndoMode {
    #[default]
    None,
    ExecutedSteps,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrySpec {
    pub attempts: u32,
    pub backoff: Duration,
    #[serde(default)]
    pub on_exit_codes: Option<Vec<i32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CommandSpec {
    /// executed as: /bin/sh -lc "<string>"
    Sh(String),
    /// execve-style argv
    Argv(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StopSpec {
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunOnce {
    pub key: RunOnceKey,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RunOnceKey {
    Auto(bool), // hash-based
    Explicit(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObserveSpec {
    #[serde(default)]
    pub logs: Option<LogSpec>,
    #[serde(default)]
    pub liveness: Option<LivenessSpec>,
    #[serde(default)]
    pub health: Option<HealthSpec>,
}

fn default_max_log_bytes() -> u64 {
    262144
}

fn default_max_log_lines() -> u32 {
    1024
}

fn default_log_timeout() -> Duration {
    Duration::from_secs(5)
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct LogLimit {
    #[serde(default = "default_max_log_bytes")]
    pub max_bytes: u64,
    #[serde(default = "default_max_log_lines")]
    pub max_lines: u32,
    #[serde(default = "default_log_timeout")]
    pub timeout: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogSpec {
    pub tail: CommandSpec,
    #[serde(default)]
    pub follow: Option<CommandSpec>,
    #[serde(default)]
    pub since: Option<Duration>,
    #[serde(default)]
    pub limits: Option<LogLimit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LivenessSpec {
    pub every: Duration,
    pub check: CommandSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthSpec {
    pub every: Duration,
    pub run: CommandSpec,
    pub fails_after: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
pub struct Workdir {
    #[serde(default)]
    pub mode: WorkdirMode,
    #[serde(default)]
    pub path: Option<String>, // if omitted: agent uses root_dir/programs/<id>
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum WorkdirMode {
    #[default]
    Persistent,
    Ephemeral,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Success,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentRevisionReport {
    pub revision_id: String,
    pub outcome: Outcome,
    pub dirty: bool,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunReport {
    pub run_id: String,
    pub revision_id: String,

    pub outcome: Outcome,

    /// If outcome is failure, set an error string.
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepReport {
    pub revision_id: String,
    pub run_id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub attempts: u32,
    #[serde(default)]
    pub exit_code: Option<i32>,
    pub report_time: u64,

    /// Whether the step ultimately succeeded.
    pub success: bool,

    /// If it failed, short error text.
    #[serde(default)]
    pub error: Option<String>,

    /// Best-effort log tail for this step only (bounded).
    #[serde(default)]
    pub log_tail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackReport {
    pub revision_id: String,
    /// Whether rollback completed successfully.
    pub success: bool,

    /// Which step indexes had undo executed (reverse order typically).
    #[serde(default)]
    pub undone_steps: Vec<u32>,

    /// Any rollback error.
    #[serde(default)]
    pub error: Option<String>,

    #[serde(default)]
    pub log_tail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunState {
    pub run_id: String,
    pub revision_id: String,
    pub healthy: Option<bool>,
    pub alive: Option<bool>,
    pub report_time: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum DeployReportKind {
    DeploymentRevisionReport(DeploymentRevisionReport),
    RunReport(RunReport),
    StepReport(StepReport),
    RollbackReport(RollbackReport),
    RunState(RunState),
}

impl DeployReportKind {
    pub fn get_revision_id(&self) -> &str {
        match self {
            DeployReportKind::DeploymentRevisionReport(r) => &r.revision_id,
            DeployReportKind::RunReport(r) => &r.revision_id,
            DeployReportKind::StepReport(r) => &r.revision_id,
            DeployReportKind::RollbackReport(r) => &r.revision_id,
            DeployReportKind::RunState(r) => &r.revision_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployReport {
    pub device_id: String,
    pub revision_id: String,
    pub kind: DeployReportKind,

    /// TTL target
    pub expires_at: Option<u64>,

    /// When the report was received/created
    pub created_at: u64,
}
