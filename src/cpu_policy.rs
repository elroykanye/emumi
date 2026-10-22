#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuStat {
    pub periods: u64,
    pub throttled_periods: u64,
    pub throttled_usec: u64,
    pub quota_percent: u16,
}

pub struct CpuQuotaPolicy {
    current_percent: u16,
    previous: Option<CpuStat>,
    busy_samples: u8,
    calm_samples: u8,
    thermal_limited: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThermalDecision {
    Keep,
    Quota(u16),
}

pub fn startup_thermal_decision(
    current_percent: u16,
    temperature_c: Option<f32>,
) -> ThermalDecision {
    let Some(temperature) = temperature_c.filter(|value| value.is_finite()) else {
        return ThermalDecision::Keep;
    };
    if temperature >= 85.0 {
        if current_percent != 50 {
            ThermalDecision::Quota(50)
        } else {
            ThermalDecision::Keep
        }
    } else if temperature >= 80.0 {
        if current_percent > 80 {
            ThermalDecision::Quota(80)
        } else {
            ThermalDecision::Keep
        }
    } else {
        ThermalDecision::Keep
    }
}

const THERMAL_LIMIT_ENTER_C: f32 = 85.0;
const THERMAL_LIMIT_EXIT_C: f32 = 80.0;

impl CpuQuotaPolicy {
    pub fn new(current_percent: u16) -> Self {
        Self {
            current_percent,
            previous: None,
            busy_samples: 0,
            calm_samples: 0,
            thermal_limited: false,
        }
    }

    pub fn observe(
        &mut self,
        sample: CpuStat,
        running_emulators: usize,
        logical_cpus: usize,
        host_cpu_percent: f32,
        temperature_c: Option<f32>,
    ) -> Option<u16> {
        if self.previous.is_none() {
            // A controller may be attached to a scope that survived an EmuMi
            // manager restart. Start from the scope's real quota instead of
            // replaying the launch default and accidentally lowering it.
            self.current_percent = sample.quota_percent;
        }
        let ceiling = quota_ceiling(running_emulators, logical_cpus);
        if self.current_percent > ceiling {
            self.current_percent = ceiling;
            self.previous = Some(sample);
            self.busy_samples = 0;
            self.calm_samples = 0;
            return Some(ceiling);
        }

        // A hot host must shed emulator load immediately. Merely blocking a
        // later increase lets an already-raised quota keep the package at its
        // thermal limit, which is exactly when the desktop and ADB become
        // unreliable. Keep this below one full host CPU so other recovery
        // work still gets scheduled.
        if temperature_c.is_some_and(|value| value.is_finite() && value >= THERMAL_LIMIT_ENTER_C) {
            self.thermal_limited = true;
        } else if self.thermal_limited
            && temperature_c.is_some_and(|value| value.is_finite() && value <= THERMAL_LIMIT_EXIT_C)
        {
            self.thermal_limited = false;
        }
        if self.thermal_limited {
            self.previous = Some(sample);
            self.busy_samples = 0;
            self.calm_samples = 0;
            let thermal_ceiling = if temperature_c.is_some_and(|value| value >= 90.0) {
                50
            } else {
                80
            };
            if self.current_percent > thermal_ceiling {
                self.current_percent = thermal_ceiling;
                return Some(thermal_ceiling);
            }
            return None;
        }

        let previous = self.previous.replace(sample)?;
        let Some(period_delta) = sample.periods.checked_sub(previous.periods) else {
            self.busy_samples = 0;
            self.calm_samples = 0;
            return None;
        };
        let Some(throttled_delta) = sample
            .throttled_periods
            .checked_sub(previous.throttled_periods)
        else {
            self.busy_samples = 0;
            self.calm_samples = 0;
            return None;
        };
        if sample
            .throttled_usec
            .checked_sub(previous.throttled_usec)
            .is_none()
        {
            self.busy_samples = 0;
            self.calm_samples = 0;
            return None;
        }
        if period_delta == 0 {
            return None;
        }

        let throttled_ratio = throttled_delta as f32 / period_delta as f32;
        let has_thermal_headroom = temperature_c.is_none_or(|value| value < 88.0);
        let has_host_headroom = host_cpu_percent < 85.0;
        if throttled_ratio >= 0.20 && has_thermal_headroom && has_host_headroom {
            self.busy_samples = self.busy_samples.saturating_add(1);
            self.calm_samples = 0;
            if self.busy_samples >= 2 && self.current_percent < ceiling {
                self.busy_samples = 0;
                self.current_percent = self.current_percent.saturating_add(10).min(ceiling);
                return Some(self.current_percent);
            }
        } else if throttled_ratio <= 0.02 {
            self.calm_samples = self.calm_samples.saturating_add(1);
            self.busy_samples = 0;
            let floor = 120.min(ceiling);
            if self.calm_samples >= 6 && self.current_percent > floor {
                self.calm_samples = 0;
                self.current_percent = self.current_percent.saturating_sub(30).max(floor);
                return Some(self.current_percent);
            }
        } else {
            self.busy_samples = 0;
            self.calm_samples = 0;
        }
        None
    }
}

fn quota_ceiling(running_emulators: usize, logical_cpus: usize) -> u16 {
    let running_emulators = running_emulators.max(1);
    let host_capacity_percent = logical_cpus.saturating_sub(2).saturating_mul(100);
    let fair_share = host_capacity_percent / running_emulators;
    u16::try_from(fair_share).unwrap_or(u16::MAX).clamp(80, 300)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stat_at(
        quota_percent: u16,
        periods: u64,
        throttled_periods: u64,
        throttled_usec: u64,
    ) -> CpuStat {
        CpuStat {
            periods,
            throttled_periods,
            throttled_usec,
            quota_percent,
        }
    }

    fn stat(periods: u64, throttled_periods: u64, throttled_usec: u64) -> CpuStat {
        stat_at(180, periods, throttled_periods, throttled_usec)
    }

    #[test]
    fn sustained_throttling_raises_quota_toward_three_cores() {
        let mut policy = CpuQuotaPolicy::new(180);

        assert_eq!(
            policy.observe(stat(100, 90, 1_000_000), 2, 8, 45.0, Some(70.0)),
            None
        );
        assert_eq!(
            policy.observe(stat(150, 135, 1_800_000), 2, 8, 45.0, Some(70.0)),
            None
        );
        assert_eq!(
            policy.observe(stat(200, 180, 2_600_000), 2, 8, 45.0, Some(70.0)),
            Some(190)
        );
        assert_eq!(
            policy.observe(stat(250, 225, 3_400_000), 2, 8, 45.0, Some(70.0)),
            None
        );
        assert_eq!(
            policy.observe(stat(300, 270, 4_200_000), 2, 8, 45.0, Some(70.0)),
            Some(200)
        );
    }

    #[test]
    fn four_emulators_share_six_cores_and_preserve_two_for_the_host() {
        let mut policy = CpuQuotaPolicy::new(180);

        assert_eq!(
            policy.observe(stat(100, 0, 0), 4, 8, 40.0, Some(65.0)),
            Some(150)
        );
        assert_eq!(
            policy.observe(stat(150, 50, 800_000), 4, 8, 40.0, Some(65.0)),
            None
        );

        assert_eq!(
            policy.observe(stat(200, 100, 1_600_000), 4, 8, 40.0, Some(65.0)),
            None
        );
    }

    #[test]
    fn calm_samples_lower_quota_slowly_without_oscillation() {
        let mut policy = CpuQuotaPolicy::new(300);
        assert_eq!(
            policy.observe(stat_at(300, 100, 0, 0), 2, 8, 30.0, Some(65.0)),
            None
        );

        for index in 1..6 {
            assert_eq!(
                policy.observe(stat(100 + index * 50, 0, 0), 2, 8, 30.0, Some(65.0)),
                None
            );
        }
        assert_eq!(
            policy.observe(stat(400, 0, 0), 2, 8, 30.0, Some(65.0)),
            Some(270)
        );
    }

    #[test]
    fn restart_adoption_uses_the_scope_quota_instead_of_the_launch_default() {
        let mut policy = CpuQuotaPolicy::new(180);

        assert_eq!(
            policy.observe(stat_at(300, 100, 90, 1_000_000), 2, 8, 45.0, Some(70.0)),
            None
        );
        assert_eq!(
            policy.observe(stat_at(300, 150, 135, 1_800_000), 2, 8, 45.0, Some(70.0)),
            None
        );
        assert_eq!(
            policy.observe(stat_at(300, 200, 180, 2_600_000), 2, 8, 45.0, Some(70.0)),
            None
        );
    }

    #[test]
    fn heat_forces_a_cap_and_saturated_host_blocks_quota_increase() {
        let mut hot = CpuQuotaPolicy::new(180);
        assert_eq!(
            hot.observe(stat(100, 90, 1_000_000), 2, 8, 45.0, Some(89.0)),
            Some(80)
        );
        assert_eq!(
            hot.observe(stat(150, 135, 1_800_000), 2, 8, 45.0, Some(89.0)),
            None
        );
        assert_eq!(
            hot.observe(stat(200, 180, 2_600_000), 2, 8, 45.0, Some(89.0)),
            None
        );

        let mut saturated = CpuQuotaPolicy::new(180);
        assert_eq!(
            saturated.observe(stat(100, 90, 1_000_000), 2, 8, 92.0, Some(70.0)),
            None
        );
        assert_eq!(
            saturated.observe(stat(150, 135, 1_800_000), 2, 8, 92.0, Some(70.0)),
            None
        );
        assert_eq!(
            saturated.observe(stat(200, 180, 2_600_000), 2, 8, 92.0, Some(70.0)),
            None
        );
    }

    #[test]
    fn extreme_heat_immediately_forces_half_a_core() {
        let mut policy = CpuQuotaPolicy::new(240);

        assert_eq!(
            policy.observe(stat_at(240, 100, 90, 1_000_000), 2, 8, 45.0, Some(99.0)),
            Some(50)
        );
        assert_eq!(
            policy.observe(stat_at(50, 150, 145, 1_800_000), 2, 8, 45.0, Some(99.0)),
            None
        );
        for (periods, throttled_periods) in [(200, 195), (250, 245), (300, 295)] {
            assert_eq!(
                policy.observe(
                    stat_at(50, periods, throttled_periods, periods * 20_000),
                    2,
                    8,
                    45.0,
                    Some(87.0),
                ),
                None,
                "emergency cap must remain latched until the host is cool"
            );
        }
    }

    #[test]
    fn manager_restart_while_still_hot_keeps_the_scope_capped() {
        let mut policy = CpuQuotaPolicy::new(180);

        assert_eq!(
            policy.observe(stat_at(240, 100, 90, 1_000_000), 2, 8, 45.0, Some(87.0)),
            Some(80)
        );
    }

    #[test]
    fn stale_or_reset_counters_do_not_trigger_a_change() {
        let mut policy = CpuQuotaPolicy::new(180);
        assert_eq!(
            policy.observe(stat(100, 80, 1_000_000), 2, 8, 40.0, Some(70.0)),
            None
        );
        assert_eq!(
            policy.observe(stat(10, 8, 100_000), 2, 8, 40.0, Some(70.0)),
            None
        );
        assert_eq!(
            policy.observe(stat(60, 53, 900_000), 2, 8, 40.0, Some(70.0)),
            None
        );
        assert_eq!(
            policy.observe(stat(110, 98, 1_700_000), 2, 8, 40.0, Some(70.0)),
            Some(190)
        );
    }

    #[test]
    fn startup_thermal_controller_uses_the_confirmed_tiers() {
        assert_eq!(
            startup_thermal_decision(80, Some(70.0)),
            ThermalDecision::Keep
        );
        assert_eq!(
            startup_thermal_decision(120, Some(70.0)),
            ThermalDecision::Keep
        );
        assert_eq!(
            startup_thermal_decision(120, Some(80.0)),
            ThermalDecision::Quota(80)
        );
        assert_eq!(
            startup_thermal_decision(80, Some(85.0)),
            ThermalDecision::Quota(50)
        );
        assert_eq!(
            startup_thermal_decision(50, Some(90.0)),
            ThermalDecision::Keep
        );
        assert_eq!(startup_thermal_decision(80, None), ThermalDecision::Keep);
        assert_eq!(
            startup_thermal_decision(50, Some(70.0)),
            ThermalDecision::Keep
        );
    }
}
