use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

const FAILURE_DELAYS_SECONDS: [u64; 6] = [5, 15, 45, 120, 300, 900];
const UNHEALTHY_SAMPLE_LIMIT: u8 = 3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DevicePhase {
    #[default]
    Stopped,
    Queued,
    Starting,
    Running,
    Recovering,
    Degraded,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct DeviceStatus {
    pub phase: DevicePhase,
    pub desired_running: bool,
    pub consecutive_failures: u32,
    pub next_retry_at: Option<u64>,
    pub last_failure: Option<String>,
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

#[derive(Clone, Debug, Default)]
struct DeviceEntry {
    status: DeviceStatus,
    unhealthy_samples: u8,
    recovery_scope: Option<String>,
    boot_started_at: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct DeviceSupervisor {
    entries: BTreeMap<String, DeviceEntry>,
}

impl DeviceSupervisor {
    pub fn new(desired: BTreeSet<String>) -> Self {
        let entries = desired
            .into_iter()
            .map(|profile| {
                (
                    profile,
                    DeviceEntry {
                        status: DeviceStatus {
                            phase: DevicePhase::Recovering,
                            desired_running: true,
                            next_retry_at: Some(0),
                            last_failure: Some("Manager resumed supervision".into()),
                            ..DeviceStatus::default()
                        },
                        ..DeviceEntry::default()
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

    #[cfg(test)]
    pub fn status(&self, profile: &str) -> &DeviceStatus {
        &self
            .entries
            .get(profile)
            .unwrap_or_else(|| panic!("missing supervisor entry for {profile}"))
            .status
    }

    pub fn request_start(&mut self, profile: &str, _now: u64) {
        let entry = self.entries.entry(profile.to_owned()).or_default();
        entry.status.desired_running = true;
        entry.status.phase = DevicePhase::Starting;
        entry.status.next_retry_at = None;
        entry.unhealthy_samples = 0;
        entry.boot_started_at = None;
    }

    pub fn mark_queued(&mut self, profile: &str) {
        let entry = self.entries.entry(profile.to_owned()).or_default();
        entry.status.desired_running = true;
        entry.status.phase = DevicePhase::Queued;
        entry.status.next_retry_at = None;
        entry.unhealthy_samples = 0;
        entry.boot_started_at = None;
    }

    pub fn request_stop(&mut self, profile: &str) {
        let entry = self.entries.entry(profile.to_owned()).or_default();
        entry.status.desired_running = false;
        entry.status.phase = DevicePhase::Stopped;
        entry.status.next_retry_at = None;
        entry.status.consecutive_failures = 0;
        entry.status.last_failure = None;
        entry.unhealthy_samples = 0;
        entry.recovery_scope = None;
        entry.boot_started_at = None;
    }

    pub fn mark_running(&mut self, profile: &str) {
        let entry = self.entries.entry(profile.to_owned()).or_default();
        entry.status.phase = DevicePhase::Running;
        entry.status.next_retry_at = None;
        entry.status.consecutive_failures = 0;
        entry.status.last_failure = None;
        entry.unhealthy_samples = 0;
        entry.recovery_scope = None;
        entry.boot_started_at = None;
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
            DevicePhase::Recovering | DevicePhase::Degraded
        ) && entry.status.next_retry_at.is_some()
        {
            return;
        }
        entry.status.consecutive_failures = entry.status.consecutive_failures.saturating_add(1);
        let delay_index = (entry.status.consecutive_failures as usize)
            .saturating_sub(1)
            .min(FAILURE_DELAYS_SECONDS.len() - 1);
        let delay = FAILURE_DELAYS_SECONDS[delay_index];
        entry.status.phase =
            if entry.status.consecutive_failures >= FAILURE_DELAYS_SECONDS.len() as u32 {
                DevicePhase::Degraded
            } else {
                DevicePhase::Recovering
            };
        entry.status.next_retry_at = Some(now.saturating_add(delay));
        entry.status.last_failure = Some(reason.into());
        entry.unhealthy_samples = 0;
        entry.recovery_scope = scope;
        entry.boot_started_at = None;
    }

    pub fn observe(&mut self, profile: &str, observation: Observation, now: u64) {
        match observation {
            Observation::Healthy => self.mark_running(profile),
            Observation::Booting { scope } => {
                let entry = self.entries.entry(profile.to_owned()).or_default();
                if !entry.status.desired_running {
                    return;
                }
                let started_at = *entry.boot_started_at.get_or_insert(now);
                entry.status.phase = DevicePhase::Starting;
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
                } else if entry.status.phase == DevicePhase::Running {
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
                    DevicePhase::Recovering | DevicePhase::Degraded
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
            entry.status.phase = DevicePhase::Starting;
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
        assert_eq!(supervisor.status("Device_1").phase, DevicePhase::Recovering);
        assert!(supervisor.due_actions(5).is_empty());
        assert_eq!(
            supervisor.due_actions(6),
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
    fn repeated_failures_back_off_and_enter_a_cooldown() {
        let mut supervisor = DeviceSupervisor::default();
        supervisor.request_start("Device_3", 0);

        let expected_delays = [5, 15, 45, 120, 300, 900];
        let mut now = 0;
        for (index, delay) in expected_delays.into_iter().enumerate() {
            supervisor.record_failure("Device_3", "native crash", None, now);
            let status = supervisor.status("Device_3");
            assert_eq!(status.next_retry_at, Some(now + delay));
            assert_eq!(status.consecutive_failures, index as u32 + 1);
            if index == expected_delays.len() - 1 {
                assert_eq!(status.phase, DevicePhase::Degraded);
            }
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

        assert_eq!(supervisor.status("Device_1").phase, DevicePhase::Running);
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
        assert_eq!(supervisor.status("Device_1").phase, DevicePhase::Running);
        supervisor.observe("Device_1", missing, 3);
        assert_eq!(supervisor.status("Device_1").phase, DevicePhase::Recovering);
    }

    #[test]
    fn booting_gets_a_real_deadline_instead_of_the_short_adb_loss_window() {
        let mut supervisor = DeviceSupervisor::new(BTreeSet::from(["Device_1".to_owned()]));
        let booting = Observation::Booting {
            scope: Some("emumi-Device_1-100.scope".into()),
        };
        supervisor.observe("Device_1", booting.clone(), 10);
        supervisor.observe("Device_1", booting.clone(), 100);
        assert_eq!(supervisor.status("Device_1").phase, DevicePhase::Starting);
        supervisor.observe("Device_1", booting, 131);
        assert_eq!(supervisor.status("Device_1").phase, DevicePhase::Recovering);
        assert!(
            supervisor
                .status("Device_1")
                .last_failure
                .as_deref()
                .unwrap()
                .contains("boot deadline")
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
        assert_eq!(status.next_retry_at, Some(125));
    }

    #[test]
    fn a_dequeued_recovery_can_recheck_stop_intent_before_spawn() {
        let mut supervisor = DeviceSupervisor::default();
        supervisor.request_start("Device_1", 0);
        supervisor.record_failure("Device_1", "native crash", None, 0);
        assert_eq!(supervisor.due_actions(5).len(), 1);

        supervisor.request_stop("Device_1");
        assert!(!supervisor.is_desired("Device_1"));
    }
}
