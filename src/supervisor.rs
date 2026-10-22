use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const FAILURE_DELAYS_SECONDS: [u64; 3] = [30, 120, 600];
const CIRCUIT_OPEN_SECONDS: u64 = 1800;
const UNHEALTHY_SAMPLE_LIMIT: u8 = 3;
const COOL_SAMPLE_LIMIT: u8 = 3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DevicePhase {
    #[default]
    Stopped,
    Queued,
    Admitting,
    Launching,
    Booting,
    Settling,
    Ready,
    ThermalHold,
    SnapshotRecovery,
    Backoff,
    HalfOpen,
    CircuitOpen,
    Stopping,
    Cleaned,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeviceStatus {
    pub phase: DevicePhase,
    pub desired_running: bool,
    pub generation: u64,
    pub stop_requested: bool,
    pub consecutive_failures: u32,
    pub next_retry_at: Option<u64>,
    pub last_failure: Option<String>,
    pub scope: Option<String>,
    pub pid: Option<u32>,
    pub cpu_quota_percent: Option<u16>,
    pub cpu_temperature_c: Option<f32>,
    pub snapshot_state: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Observation {
    Healthy,
    Booting { scope: Option<String> },
    Exited { scope: Option<String> },
    Unresponsive { scope: Option<String> },
    Absent { scope: Option<String> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SupervisorAction {
    Recover {
        profile: String,
        scope: Option<String>,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PersistedDeviceState {
    status: DeviceStatus,
    #[serde(default)]
    unhealthy_samples: u8,
    #[serde(default)]
    cool_samples: u8,
    #[serde(default)]
    recovery_scope: Option<String>,
    #[serde(default)]
    boot_started_at: Option<u64>,
    #[serde(default)]
    manual_admission: bool,
}

#[derive(Clone, Debug, Default)]
pub struct DeviceSupervisor {
    entries: BTreeMap<String, PersistedDeviceState>,
}

impl DeviceSupervisor {
    pub fn new(desired: BTreeSet<String>) -> Self {
        let entries = desired
            .into_iter()
            .map(|profile| {
                (
                    profile,
                    PersistedDeviceState {
                        status: DeviceStatus {
                            phase: DevicePhase::Backoff,
                            desired_running: true,
                            next_retry_at: Some(0),
                            last_failure: Some("Manager resumed supervision".into()),
                            ..DeviceStatus::default()
                        },
                        ..PersistedDeviceState::default()
                    },
                )
            })
            .collect();
        Self { entries }
    }

    pub fn desired_profiles(&self) -> BTreeSet<String> {
        self.entries
            .iter()
            .filter(|(_, entry)| entry.status.desired_running)
            .map(|(name, _)| name.clone())
            .collect()
    }

    pub fn statuses(&self) -> BTreeMap<String, DeviceStatus> {
        self.entries
            .iter()
            .map(|(name, entry)| (name.clone(), entry.status.clone()))
            .collect()
    }

    pub fn is_desired(&self, profile: &str) -> bool {
        self.entries
            .get(profile)
            .is_some_and(|entry| entry.status.desired_running)
    }

    pub fn generation(&self, profile: &str) -> Option<u64> {
        self.entries
            .get(profile)
            .map(|entry| entry.status.generation)
    }

    pub fn status_snapshot(&self, profile: &str) -> Option<DeviceStatus> {
        self.entries.get(profile).map(|entry| entry.status.clone())
    }

    pub fn persisted_states(&self) -> BTreeMap<String, PersistedDeviceState> {
        self.entries.clone()
    }

    pub fn restore(states: BTreeMap<String, PersistedDeviceState>) -> Self {
        Self { entries: states }
    }

    #[cfg(test)]
    pub fn status(&self, profile: &str) -> &DeviceStatus {
        &self
            .entries
            .get(profile)
            .unwrap_or_else(|| panic!("missing supervisor entry for {profile}"))
            .status
    }

    pub fn begin_admission(
        &mut self,
        profile: &str,
        _now: u64,
        manual: bool,
    ) -> Result<u64, String> {
        let entry = self.entries.entry(profile.to_owned()).or_default();
        if entry.status.phase == DevicePhase::ThermalHold {
            return Err("Android is in Thermal Hold until three readings are below 75 C".into());
        }
        if entry.status.desired_running
            && matches!(
                entry.status.phase,
                DevicePhase::Admitting
                    | DevicePhase::Launching
                    | DevicePhase::Booting
                    | DevicePhase::Settling
                    | DevicePhase::HalfOpen
            )
        {
            return Err("Android already has an active start generation".into());
        }
        entry.status.generation = entry.status.generation.wrapping_add(1).max(1);
        entry.status.desired_running = true;
        entry.status.stop_requested = false;
        entry.status.phase = DevicePhase::Admitting;
        entry.status.next_retry_at = None;
        entry.manual_admission = manual;
        entry.unhealthy_samples = 0;
        entry.cool_samples = 0;
        entry.boot_started_at = None;
        Ok(entry.status.generation)
    }

    pub fn admission_succeeded(&mut self, profile: &str, generation: u64) -> bool {
        let Some(entry) = self.entries.get_mut(profile) else {
            return false;
        };
        if entry.status.generation != generation
            || !entry.status.desired_running
            || entry.status.stop_requested
        {
            return false;
        }
        if entry.manual_admission {
            entry.status.consecutive_failures = 0;
            entry.status.last_failure = None;
        }
        entry.manual_admission = false;
        true
    }

    pub fn reject_admission(&mut self, profile: &str, reason: impl Into<String>, now: u64) {
        let entry = self.entries.entry(profile.to_owned()).or_default();
        entry.status.phase = DevicePhase::Backoff;
        entry.status.last_failure = Some(reason.into());
        entry.status.next_retry_at = Some(now.saturating_add(FAILURE_DELAYS_SECONDS[0]));
        entry.manual_admission = false;
    }

    #[cfg(test)]
    pub fn request_start(&mut self, profile: &str, now: u64) {
        let _ = self.begin_admission(profile, now, true);
    }

    pub fn mark_queued(&mut self, profile: &str) {
        let entry = self.entries.entry(profile.to_owned()).or_default();
        entry.status.desired_running = true;
        entry.status.phase = DevicePhase::Queued;
        entry.status.stop_requested = false;
        entry.status.next_retry_at = None;
        entry.unhealthy_samples = 0;
        entry.boot_started_at = None;
    }

    pub fn request_stop(&mut self, profile: &str) {
        let entry = self.entries.entry(profile.to_owned()).or_default();
        entry.status.desired_running = false;
        entry.status.phase = DevicePhase::Stopping;
        entry.status.stop_requested = true;
        entry.status.generation = entry.status.generation.wrapping_add(1).max(1);
        entry.status.next_retry_at = None;
        entry.status.consecutive_failures = 0;
        entry.status.last_failure = None;
        entry.status.snapshot_state = None;
        entry.unhealthy_samples = 0;
        entry.recovery_scope = None;
        entry.boot_started_at = None;
    }

    pub fn mark_cleaned(&mut self, profile: &str) -> bool {
        let entry = self.entries.entry(profile.to_owned()).or_default();
        if entry.status.desired_running || !entry.status.stop_requested {
            return false;
        }
        entry.status.phase = DevicePhase::Cleaned;
        entry.status.scope = None;
        entry.status.pid = None;
        entry.recovery_scope = None;
        entry.status.phase = DevicePhase::Stopped;
        true
    }

    pub fn cleanup_is_current(
        &self,
        profile: &str,
        generation: u64,
        scope: Option<&str>,
        pid: Option<u32>,
    ) -> bool {
        self.entries.get(profile).is_some_and(|entry| {
            entry.status.generation == generation
                && !entry.status.desired_running
                && entry.status.stop_requested
                && entry.status.scope.as_deref() == scope
                && entry.status.pid == pid
        })
    }

    pub fn complete_cleanup(
        &mut self,
        profile: &str,
        generation: u64,
        scope: Option<&str>,
        pid: Option<u32>,
    ) -> bool {
        if !self.cleanup_is_current(profile, generation, scope, pid) {
            return false;
        }
        self.mark_cleaned(profile)
    }

    pub fn mark_cleanup_failed(
        &mut self,
        profile: &str,
        generation: u64,
        scope: Option<&str>,
        pid: Option<u32>,
        reason: impl Into<String>,
    ) -> bool {
        if !self.cleanup_is_current(profile, generation, scope, pid) {
            return false;
        }
        let entry = self.entries.get_mut(profile).expect("checked entry");
        entry.status.phase = DevicePhase::Stopping;
        entry.status.last_failure = Some(reason.into());
        true
    }

    pub fn confirm_ownership_gone(
        &mut self,
        profile: &str,
        generation: u64,
        scope: Option<&str>,
        pid: Option<u32>,
    ) -> bool {
        let Some(entry) = self.entries.get_mut(profile) else {
            return false;
        };
        if entry.status.generation != generation
            || entry.status.scope.as_deref() != scope
            || entry.status.pid != pid
        {
            return false;
        }
        entry.status.scope = None;
        entry.status.pid = None;
        entry.recovery_scope = None;
        true
    }

    pub fn adopt_scope(&mut self, profile: &str, scope: String) -> bool {
        let Some(entry) = self.entries.get_mut(profile) else {
            return false;
        };
        if !entry.status.desired_running {
            return false;
        }
        match entry.status.scope.as_deref() {
            Some(existing) => existing == scope,
            None => {
                entry.status.scope = Some(scope.clone());
                entry.recovery_scope = Some(scope);
                true
            }
        }
    }

    pub fn mark_launching(
        &mut self,
        profile: &str,
        generation: u64,
        scope: String,
        pid: u32,
    ) -> bool {
        let Some(entry) = self.entries.get_mut(profile) else {
            return false;
        };
        if entry.status.generation != generation
            || !entry.status.desired_running
            || entry.status.stop_requested
        {
            return false;
        }
        entry.status.phase = DevicePhase::Launching;
        entry.status.scope = Some(scope.clone());
        entry.status.pid = Some(pid);
        entry.recovery_scope = Some(scope);
        true
    }

    pub fn mark_booting(&mut self, profile: &str, generation: u64) -> bool {
        self.set_generation_phase(profile, generation, DevicePhase::Booting)
    }

    pub fn mark_settling(&mut self, profile: &str, generation: u64) -> bool {
        self.set_generation_phase(profile, generation, DevicePhase::Settling)
    }

    fn set_generation_phase(&mut self, profile: &str, generation: u64, phase: DevicePhase) -> bool {
        let Some(entry) = self.entries.get_mut(profile) else {
            return false;
        };
        if entry.status.generation != generation
            || !entry.status.desired_running
            || entry.status.stop_requested
        {
            return false;
        }
        entry.status.phase = phase;
        true
    }

    pub fn complete_ready(&mut self, profile: &str, generation: u64) -> bool {
        if !self.set_generation_phase(profile, generation, DevicePhase::Ready) {
            return false;
        }
        let entry = self.entries.get_mut(profile).expect("entry exists");
        entry.status.next_retry_at = None;
        entry.status.consecutive_failures = 0;
        entry.status.last_failure = None;
        entry.status.snapshot_state = None;
        entry.unhealthy_samples = 0;
        entry.cool_samples = 0;
        entry.boot_started_at = None;
        true
    }

    pub fn mark_running(&mut self, profile: &str) {
        let generation = self
            .entries
            .entry(profile.to_owned())
            .or_default()
            .status
            .generation;
        let _ = self.complete_ready(profile, generation);
    }

    pub fn enter_thermal_hold(
        &mut self,
        profile: &str,
        generation: u64,
        temperature: f32,
        now: u64,
    ) -> bool {
        let Some(entry) = self.entries.get_mut(profile) else {
            return false;
        };
        if entry.status.generation != generation || !entry.status.desired_running {
            return false;
        }
        entry.status.phase = DevicePhase::ThermalHold;
        entry.status.cpu_temperature_c = Some(temperature);
        entry.status.last_failure = Some(format!("Thermal stop at {temperature:.1} C"));
        entry.status.next_retry_at = Some(now.saturating_add(30));
        entry.cool_samples = 0;
        true
    }

    pub fn schedule_snapshot_recovery(&mut self, profile: &str, generation: u64, now: u64) -> bool {
        let Some(entry) = self.entries.get_mut(profile) else {
            return false;
        };
        if entry.status.generation != generation
            || !entry.status.desired_running
            || entry.status.snapshot_state.as_deref() == Some("cold_retry_used")
        {
            return false;
        }
        entry.status.phase = DevicePhase::SnapshotRecovery;
        entry.status.snapshot_state = Some("cold_retry_used".into());
        entry.status.last_failure = Some("Failed default_boot snapshot quarantined".into());
        entry.status.next_retry_at = Some(now);
        entry.recovery_scope = entry.status.scope.clone();
        true
    }

    pub fn observe_cool_sample(&mut self, profile: &str, temperature: f32, now: u64) -> bool {
        let Some(entry) = self.entries.get_mut(profile) else {
            return false;
        };
        entry.status.cpu_temperature_c = Some(temperature);
        if entry.status.phase != DevicePhase::ThermalHold {
            return false;
        }
        if temperature < 75.0 {
            entry.cool_samples = entry.cool_samples.saturating_add(1);
        } else {
            entry.cool_samples = 0;
        }
        if entry.cool_samples < COOL_SAMPLE_LIMIT {
            return false;
        }
        entry.status.phase = DevicePhase::Backoff;
        entry.status.next_retry_at = Some(now);
        entry.cool_samples = 0;
        true
    }

    pub fn update_telemetry(
        &mut self,
        profile: &str,
        generation: u64,
        quota_percent: u16,
        temperature_c: Option<f32>,
    ) -> bool {
        let Some(entry) = self.entries.get_mut(profile) else {
            return false;
        };
        if entry.status.generation != generation || !entry.status.desired_running {
            return false;
        }
        entry.status.cpu_quota_percent = Some(quota_percent);
        entry.status.cpu_temperature_c = temperature_c;
        true
    }

    pub fn record_failure(
        &mut self,
        profile: &str,
        reason: impl Into<String>,
        scope: Option<String>,
        now: u64,
    ) {
        let entry = self.entries.entry(profile.to_owned()).or_default();
        if !entry.status.desired_running {
            return;
        }
        if matches!(
            entry.status.phase,
            DevicePhase::Backoff | DevicePhase::CircuitOpen
        ) && entry.status.next_retry_at.is_some()
        {
            return;
        }
        entry.status.consecutive_failures = entry.status.consecutive_failures.saturating_add(1);
        let circuit = entry.status.consecutive_failures > FAILURE_DELAYS_SECONDS.len() as u32;
        let delay = if circuit {
            CIRCUIT_OPEN_SECONDS
        } else {
            FAILURE_DELAYS_SECONDS[(entry.status.consecutive_failures - 1) as usize]
        };
        entry.status.phase = if circuit {
            DevicePhase::CircuitOpen
        } else {
            DevicePhase::Backoff
        };
        entry.status.next_retry_at = Some(now.saturating_add(delay));
        entry.status.last_failure = Some(reason.into());
        entry.unhealthy_samples = 0;
        entry.recovery_scope = scope;
        entry.boot_started_at = None;
    }

    pub fn observe(&mut self, profile: &str, observation: Observation, now: u64) {
        match observation {
            Observation::Healthy => {
                let startup_phase = self.entries.get(profile).is_some_and(|entry| {
                    matches!(
                        entry.status.phase,
                        DevicePhase::Admitting
                            | DevicePhase::Launching
                            | DevicePhase::Booting
                            | DevicePhase::Settling
                            | DevicePhase::HalfOpen
                            | DevicePhase::ThermalHold
                    )
                });
                if !startup_phase {
                    self.mark_running(profile);
                }
            }
            Observation::Booting { scope } => {
                let entry = self.entries.entry(profile.to_owned()).or_default();
                if !entry.status.desired_running {
                    return;
                }
                if matches!(
                    entry.status.phase,
                    DevicePhase::Backoff
                        | DevicePhase::CircuitOpen
                        | DevicePhase::SnapshotRecovery
                        | DevicePhase::ThermalHold
                        | DevicePhase::Stopping
                ) && entry.status.next_retry_at.is_some()
                    && (entry.status.scope.is_some() || entry.recovery_scope.is_some())
                {
                    return;
                }
                let started_at = *entry.boot_started_at.get_or_insert(now);
                entry.status.phase = DevicePhase::Booting;
                entry.status.scope = scope.clone();
                entry.recovery_scope = scope.clone();
                if now.saturating_sub(started_at) >= 120 {
                    self.record_failure(
                        profile,
                        "Emulator exceeded its 120 second boot deadline",
                        scope,
                        now,
                    );
                }
            }
            Observation::Exited { scope } => {
                self.record_failure(profile, "Native emulator process exited", scope, now)
            }
            Observation::Unresponsive { scope } => {
                let entry = self.entries.entry(profile.to_owned()).or_default();
                if !entry.status.desired_running {
                    return;
                }
                entry.unhealthy_samples = entry.unhealthy_samples.saturating_add(1);
                if entry.unhealthy_samples >= UNHEALTHY_SAMPLE_LIMIT {
                    self.record_failure(
                        profile,
                        "Emulator was unavailable through ADB for three health samples",
                        scope,
                        now,
                    );
                }
            }
            Observation::Absent { scope } => {
                let entry = self.entries.entry(profile.to_owned()).or_default();
                if !entry.status.desired_running {
                    entry.status.phase = DevicePhase::Stopped;
                    entry.status.next_retry_at = None;
                } else if entry.status.phase == DevicePhase::Ready {
                    entry.unhealthy_samples = entry.unhealthy_samples.saturating_add(1);
                    if entry.unhealthy_samples >= UNHEALTHY_SAMPLE_LIMIT {
                        self.record_failure(profile, "Emulator disappeared from ADB", scope, now);
                    }
                }
            }
        }
    }

    pub fn due_actions(&mut self, now: u64) -> Vec<SupervisorAction> {
        let mut actions = Vec::new();
        for (profile, entry) in &mut self.entries {
            if !entry.status.desired_running
                || !matches!(
                    entry.status.phase,
                    DevicePhase::Backoff | DevicePhase::CircuitOpen | DevicePhase::SnapshotRecovery
                )
                || !entry
                    .status
                    .next_retry_at
                    .is_some_and(|retry_at| retry_at <= now)
            {
                continue;
            }
            actions.push(SupervisorAction::Recover {
                profile: profile.clone(),
                scope: entry.recovery_scope.take(),
            });
            entry.status.generation = entry.status.generation.wrapping_add(1).max(1);
            entry.status.stop_requested = false;
            entry.status.phase = if entry.status.phase == DevicePhase::CircuitOpen {
                DevicePhase::HalfOpen
            } else {
                DevicePhase::Admitting
            };
            entry.status.next_retry_at = None;
            entry.unhealthy_samples = 0;
            entry.boot_started_at = None;
        }
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::{DevicePhase, DeviceSupervisor, Observation, SupervisorAction};
    use std::collections::BTreeSet;

    #[test]
    fn unexpected_exit_is_recovered_but_an_explicit_stop_is_not() {
        let mut supervisor = DeviceSupervisor::new(BTreeSet::from(["Device_1".to_owned()]));
        supervisor.observe("Device_1", Observation::Healthy, 0);

        supervisor.observe(
            "Device_1",
            Observation::Exited {
                scope: Some("emumi-Device_1-100.scope".into()),
            },
            1,
        );
        assert_eq!(supervisor.status("Device_1").phase, DevicePhase::Backoff);
        assert!(supervisor.due_actions(30).is_empty());
        assert_eq!(
            supervisor.due_actions(31),
            vec![SupervisorAction::Recover {
                profile: "Device_1".into(),
                scope: Some("emumi-Device_1-100.scope".into()),
            }]
        );

        supervisor.request_stop("Device_1");
        supervisor.observe("Device_1", Observation::Absent { scope: None }, 10);
        assert_eq!(supervisor.status("Device_1").phase, DevicePhase::Stopped);
        assert!(!supervisor.status("Device_1").desired_running);
        assert!(supervisor.due_actions(u64::MAX).is_empty());
    }

    #[test]
    fn repeated_failures_back_off_and_enter_a_circuit() {
        let mut supervisor = DeviceSupervisor::default();
        supervisor.request_start("Device_3", 0);

        let expected_delays = [30, 120, 600, 1800];
        let mut now = 0;
        for (index, delay) in expected_delays.into_iter().enumerate() {
            supervisor.record_failure("Device_3", "native crash", None, now);
            let status = supervisor.status("Device_3");
            assert_eq!(status.next_retry_at, Some(now + delay));
            assert_eq!(status.consecutive_failures, index as u32 + 1);
            assert_eq!(
                status.phase,
                if index == expected_delays.len() - 1 {
                    DevicePhase::CircuitOpen
                } else {
                    DevicePhase::Backoff
                }
            );
            now += delay;
            assert_eq!(supervisor.due_actions(now).len(), 1);
        }
    }

    #[test]
    fn persisted_desired_devices_are_adopted_or_recovered_without_duplicates() {
        let mut supervisor = DeviceSupervisor::new(BTreeSet::from([
            "Device_1".to_owned(),
            "Device_3".to_owned(),
        ]));

        supervisor.observe("Device_1", Observation::Healthy, 100);
        supervisor.observe("Device_3", Observation::Absent { scope: None }, 100);

        assert_eq!(supervisor.status("Device_1").phase, DevicePhase::Ready);
        assert_eq!(
            supervisor.due_actions(100),
            vec![SupervisorAction::Recover {
                profile: "Device_3".into(),
                scope: None,
            }]
        );
        assert!(supervisor.due_actions(100).is_empty());
    }

    #[test]
    fn transient_adb_loss_requires_three_samples_before_cleanup() {
        let mut supervisor = DeviceSupervisor::new(BTreeSet::from(["Device_1".to_owned()]));
        supervisor.observe("Device_1", Observation::Healthy, 0);
        let missing = Observation::Unresponsive {
            scope: Some("emumi-Device_1-100.scope".into()),
        };

        supervisor.observe("Device_1", missing.clone(), 1);
        supervisor.observe("Device_1", missing.clone(), 2);
        assert_eq!(supervisor.status("Device_1").phase, DevicePhase::Ready);
        supervisor.observe("Device_1", missing, 3);
        assert_eq!(supervisor.status("Device_1").phase, DevicePhase::Backoff);
    }

    #[test]
    fn booting_gets_a_real_deadline_instead_of_the_short_adb_loss_window() {
        let mut supervisor = DeviceSupervisor::new(BTreeSet::from(["Device_1".to_owned()]));
        let booting = Observation::Booting {
            scope: Some("emumi-Device_1-100.scope".into()),
        };
        supervisor.observe("Device_1", booting.clone(), 10);
        supervisor.observe("Device_1", booting.clone(), 100);
        assert_eq!(supervisor.status("Device_1").phase, DevicePhase::Booting);
        supervisor.observe("Device_1", booting, 131);
        assert_eq!(supervisor.status("Device_1").phase, DevicePhase::Backoff);
        assert!(
            supervisor
                .status("Device_1")
                .last_failure
                .as_deref()
                .unwrap()
                .contains("boot deadline")
        );
        supervisor.observe(
            "Device_1",
            Observation::Booting {
                scope: Some("emumi-Device_1-100.scope".into()),
            },
            132,
        );
        assert_eq!(supervisor.status("Device_1").phase, DevicePhase::Backoff);
        assert_eq!(
            supervisor.due_actions(161),
            vec![SupervisorAction::Recover {
                profile: "Device_1".into(),
                scope: Some("emumi-Device_1-100.scope".into()),
            }]
        );
    }

    #[test]
    fn queued_state_is_visible_without_consuming_a_failure() {
        let mut supervisor = DeviceSupervisor::default();
        supervisor.request_start("Device_4", 0);
        supervisor.mark_queued("Device_4");
        let status = supervisor.status("Device_4");
        assert_eq!(status.phase, DevicePhase::Queued);
        assert_eq!(status.consecutive_failures, 0);
        assert!(status.next_retry_at.is_none());
    }

    #[test]
    fn the_same_unresolved_failure_is_not_counted_twice() {
        let mut supervisor = DeviceSupervisor::default();
        supervisor.request_start("Device_1", 0);
        supervisor.record_failure("Device_1", "boot deadline", None, 120);
        supervisor.record_failure("Device_1", "boot deadline", None, 121);

        let status = supervisor.status("Device_1");
        assert_eq!(status.consecutive_failures, 1);
        assert_eq!(status.next_retry_at, Some(150));
    }

    #[test]
    fn a_dequeued_recovery_can_recheck_stop_intent_before_spawn() {
        let mut supervisor = DeviceSupervisor::default();
        supervisor.request_start("Device_1", 0);
        supervisor.record_failure("Device_1", "native crash", None, 0);
        assert_eq!(supervisor.due_actions(30).len(), 1);

        supervisor.request_stop("Device_1");
        assert!(!supervisor.is_desired("Device_1"));
    }

    #[test]
    fn launch_generation_moves_through_explicit_phases_and_rejects_late_completion() {
        let mut supervisor = DeviceSupervisor::default();
        let first = supervisor.begin_admission("Device_4", 10, true).unwrap();
        assert_eq!(supervisor.status("Device_4").phase, DevicePhase::Admitting);
        supervisor.mark_launching("Device_4", first, "emumi-Device_4-10.scope".into(), 1234);
        assert_eq!(supervisor.status("Device_4").phase, DevicePhase::Launching);
        supervisor.mark_booting("Device_4", first);
        assert_eq!(supervisor.status("Device_4").phase, DevicePhase::Booting);
        supervisor.mark_settling("Device_4", first);
        assert_eq!(supervisor.status("Device_4").phase, DevicePhase::Settling);

        supervisor.request_stop("Device_4");
        assert!(!supervisor.complete_ready("Device_4", first));
        assert!(!supervisor.status("Device_4").desired_running);
        assert!(supervisor.status("Device_4").stop_requested);
    }

    #[test]
    fn failures_use_three_backoffs_then_a_thirty_minute_single_probe_circuit() {
        let mut supervisor = DeviceSupervisor::default();
        supervisor.begin_admission("Device_3", 0, true).unwrap();
        let expected = [30, 120, 600];
        let mut now = 0;
        for delay in expected {
            supervisor.record_failure("Device_3", "crash", None, now);
            assert_eq!(supervisor.status("Device_3").phase, DevicePhase::Backoff);
            assert_eq!(
                supervisor.status("Device_3").next_retry_at,
                Some(now + delay)
            );
            now += delay;
            assert_eq!(supervisor.due_actions(now).len(), 1);
        }

        supervisor.record_failure("Device_3", "crash", None, now);
        assert_eq!(
            supervisor.status("Device_3").phase,
            DevicePhase::CircuitOpen
        );
        assert_eq!(
            supervisor.status("Device_3").next_retry_at,
            Some(now + 1800)
        );
        assert!(supervisor.due_actions(now + 1799).is_empty());
        assert_eq!(supervisor.due_actions(now + 1800).len(), 1);
        assert_eq!(supervisor.status("Device_3").phase, DevicePhase::HalfOpen);
        assert!(supervisor.due_actions(now + 1800).is_empty());
    }

    #[test]
    fn manual_start_cannot_erase_failures_until_admission_succeeds() {
        let mut supervisor = DeviceSupervisor::default();
        supervisor.begin_admission("Device_3", 0, true).unwrap();
        supervisor.record_failure("Device_3", "crash", None, 0);
        assert_eq!(supervisor.status("Device_3").consecutive_failures, 1);

        supervisor.reject_admission("Device_3", "host too hot", 5);
        assert_eq!(supervisor.status("Device_3").consecutive_failures, 1);
        let generation = supervisor.begin_admission("Device_3", 6, true).unwrap();
        supervisor.admission_succeeded("Device_3", generation);
        assert_eq!(supervisor.status("Device_3").consecutive_failures, 0);
    }

    #[test]
    fn thermal_hold_is_persistable_and_requires_three_cool_samples() {
        let mut supervisor = DeviceSupervisor::default();
        let generation = supervisor.begin_admission("Device_3", 0, true).unwrap();
        supervisor.enter_thermal_hold("Device_3", generation, 96.0, 10);
        assert_eq!(
            supervisor.status("Device_3").phase,
            DevicePhase::ThermalHold
        );

        let persisted = supervisor.persisted_states();
        let mut restored = DeviceSupervisor::restore(persisted);
        assert_eq!(restored.status("Device_3").phase, DevicePhase::ThermalHold);
        assert!(!restored.observe_cool_sample("Device_3", 74.0, 20));
        assert!(!restored.observe_cool_sample("Device_3", 74.0, 21));
        assert!(restored.observe_cool_sample("Device_3", 74.0, 22));
        assert_eq!(restored.status("Device_3").phase, DevicePhase::Backoff);
    }

    #[test]
    fn snapshot_recovery_allows_exactly_one_cold_retry_per_failure_streak() {
        let mut supervisor = DeviceSupervisor::default();
        let generation = supervisor.begin_admission("Device_4", 0, true).unwrap();
        assert!(supervisor.schedule_snapshot_recovery("Device_4", generation, 10));
        assert_eq!(
            supervisor.status("Device_4").phase,
            DevicePhase::SnapshotRecovery
        );
        assert!(!supervisor.schedule_snapshot_recovery("Device_4", generation, 11));
        assert_eq!(supervisor.due_actions(10).len(), 1);
        let retry_generation = supervisor.status("Device_4").generation;
        assert!(!supervisor.schedule_snapshot_recovery("Device_4", retry_generation, 12));
        assert!(supervisor.complete_ready("Device_4", retry_generation));
        assert_eq!(supervisor.status("Device_4").snapshot_state, None);
        assert!(supervisor.schedule_snapshot_recovery("Device_4", retry_generation, 13));
    }

    #[test]
    fn stale_stop_completion_cannot_clear_a_new_launch_generation() {
        let mut supervisor = DeviceSupervisor::default();
        let old = supervisor.begin_admission("Device_4", 0, true).unwrap();
        supervisor.mark_launching("Device_4", old, "emumi-Device_4-100.scope".into(), 123);
        supervisor.request_stop("Device_4");
        let new = supervisor.begin_admission("Device_4", 1, true).unwrap();
        supervisor.mark_launching("Device_4", new, "emumi-Device_4-200.scope".into(), 456);
        supervisor.mark_booting("Device_4", new);

        assert!(!supervisor.mark_cleaned("Device_4"));
        assert_eq!(supervisor.status("Device_4").phase, DevicePhase::Booting);
        assert_eq!(
            supervisor.status("Device_4").scope.as_deref(),
            Some("emumi-Device_4-200.scope")
        );
    }

    #[test]
    fn healthy_observation_cannot_bypass_startup_settling() {
        let mut supervisor = DeviceSupervisor::default();
        let generation = supervisor.begin_admission("Device_4", 0, true).unwrap();
        supervisor.mark_booting("Device_4", generation);
        supervisor.observe("Device_4", Observation::Healthy, 1);
        assert_eq!(supervisor.status("Device_4").phase, DevicePhase::Booting);
        supervisor.mark_settling("Device_4", generation);
        supervisor.observe("Device_4", Observation::Healthy, 2);
        assert_eq!(supervisor.status("Device_4").phase, DevicePhase::Settling);
    }

    #[test]
    fn thermal_hold_rejects_health_and_manual_start_until_cooldown() {
        let mut supervisor = DeviceSupervisor::default();
        let generation = supervisor.begin_admission("Device_4", 0, true).unwrap();
        supervisor.mark_launching(
            "Device_4",
            generation,
            "emumi-Device_4-123.scope".into(),
            12345,
        );
        supervisor.enter_thermal_hold("Device_4", generation, 95.0, 1);

        supervisor.observe("Device_4", Observation::Healthy, 2);
        assert_eq!(
            supervisor.status("Device_4").phase,
            DevicePhase::ThermalHold
        );
        assert!(supervisor.begin_admission("Device_4", 2, true).is_err());
        supervisor.observe(
            "Device_4",
            Observation::Booting {
                scope: Some("emumi-Device_4-stale.scope".into()),
            },
            3,
        );
        assert_eq!(
            supervisor.status("Device_4").phase,
            DevicePhase::ThermalHold
        );
        assert!(!supervisor.observe_cool_sample("Device_4", 74.0, 3));
        assert!(!supervisor.observe_cool_sample("Device_4", 74.0, 4));
        assert!(supervisor.observe_cool_sample("Device_4", 74.0, 5));
        assert_eq!(supervisor.status("Device_4").phase, DevicePhase::Backoff);
    }

    #[test]
    fn snapshot_recovery_retains_scope_and_pid_until_cleanup_proof() {
        let mut supervisor = DeviceSupervisor::default();
        let generation = supervisor.begin_admission("Device_4", 0, true).unwrap();
        supervisor.mark_launching(
            "Device_4",
            generation,
            "emumi-Device_4-123.scope".into(),
            12345,
        );
        supervisor.mark_booting("Device_4", generation);

        assert!(supervisor.schedule_snapshot_recovery("Device_4", generation, 1));
        assert_eq!(
            supervisor.status("Device_4").scope.as_deref(),
            Some("emumi-Device_4-123.scope")
        );
        assert_eq!(supervisor.status("Device_4").pid, Some(12345));
    }
}
