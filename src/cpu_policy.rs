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
}

impl CpuQuotaPolicy {
    pub fn new(current_percent: u16) -> Self {
        Self {
            current_percent,
            previous: None,
            busy_samples: 0,
            calm_samples: 0,
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
                self.current_percent = self.current_percent.saturating_add(60).min(ceiling);
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
            Some(240)
        );
        assert_eq!(
            policy.observe(stat(250, 225, 3_400_000), 2, 8, 45.0, Some(70.0)),
            None
        );
        assert_eq!(
            policy.observe(stat(300, 270, 4_200_000), 2, 8, 45.0, Some(70.0)),
            Some(300)
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
    fn heat_or_saturated_host_blocks_quota_increase() {
        let mut hot = CpuQuotaPolicy::new(180);
        assert_eq!(
            hot.observe(stat(100, 90, 1_000_000), 2, 8, 45.0, Some(89.0)),
            None
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
            Some(240)
        );
    }
}
