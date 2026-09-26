use crate::model::HostStats;
use std::fs;

#[derive(Default)]
pub struct HostMonitor {
    previous_cpu: Option<(u64, u64)>,
}

impl HostMonitor {
    pub fn sample(&mut self) -> HostStats {
        let (cpu_percent, next_cpu) = self.cpu_sample();
        self.previous_cpu = next_cpu;
        let (memory_used_bytes, memory_total_bytes) = memory_sample();
        HostStats {
            cpu_percent,
            memory_used_bytes,
            memory_total_bytes,
        }
    }

    fn cpu_sample(&self) -> (f32, Option<(u64, u64)>) {
        let Some(line) = fs::read_to_string("/proc/stat")
            .ok()
            .and_then(|text| text.lines().next().map(str::to_owned))
        else {
            return (0.0, None);
        };
        let numbers: Vec<u64> = line
            .split_whitespace()
            .skip(1)
            .filter_map(|part| part.parse().ok())
            .collect();
        let total: u64 = numbers.iter().sum();
        let idle = numbers.get(3).copied().unwrap_or(0) + numbers.get(4).copied().unwrap_or(0);
        let usage = self
            .previous_cpu
            .and_then(|(old_total, old_idle)| {
                let total_delta = total.saturating_sub(old_total);
                let idle_delta = idle.saturating_sub(old_idle);
                (total_delta > 0)
                    .then(|| 100.0 * (total_delta - idle_delta) as f32 / total_delta as f32)
            })
            .unwrap_or(0.0);
        (usage, Some((total, idle)))
    }
}

fn memory_sample() -> (u64, u64) {
    let Ok(text) = fs::read_to_string("/proc/meminfo") else {
        return (0, 0);
    };
    let mut total_kb = 0;
    let mut available_kb = 0;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("MemTotal:") {
            total_kb = parse_kb(value);
        } else if let Some(value) = line.strip_prefix("MemAvailable:") {
            available_kb = parse_kb(value);
        }
    }
    ((total_kb - available_kb) * 1024, total_kb * 1024)
}

fn parse_kb(value: &str) -> u64 {
    value
        .split_whitespace()
        .next()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}
