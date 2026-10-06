use crate::model::{AndroidProfile, DeviceHostStats, HostStats};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub struct HostMonitor {
    previous_cpu: Option<(u64, u64)>,
    proc_root: PathBuf,
    sys_root: PathBuf,
}

impl Default for HostMonitor {
    fn default() -> Self {
        Self {
            previous_cpu: None,
            proc_root: PathBuf::from("/proc"),
            sys_root: PathBuf::from("/sys"),
        }
    }
}

pub fn sample_devices(profiles: &[AndroidProfile]) -> Vec<DeviceHostStats> {
    profiles
        .iter()
        .filter_map(|profile| {
            let serial = profile.running_serial.as_ref()?;
            Some(sample_device(&profile.name, serial))
        })
        .collect()
}

fn sample_device(profile_name: &str, serial: &str) -> DeviceHostStats {
    let pid = find_emulator_pid(Path::new("/proc"), profile_name);
    let mut sample = DeviceHostStats {
        profile_name: profile_name.to_owned(),
        serial: serial.to_owned(),
        pid,
        ..DeviceHostStats::default()
    };
    let Some(pid) = pid else {
        return sample;
    };
    let process = PathBuf::from("/proc").join(pid.to_string());
    let status = fs::read_to_string(process.join("status")).unwrap_or_default();
    sample.rss_bytes = status_kb(&status, "VmRSS:") * 1024;
    sample.vm_swap_bytes = status_kb(&status, "VmSwap:") * 1024;
    let rollup = fs::read_to_string(process.join("smaps_rollup")).unwrap_or_default();
    sample.pss_bytes = status_kb(&rollup, "Pss:") * 1024;

    let Some(relative) = process_cgroup(&process) else {
        return sample;
    };
    let Some(scope_name) = relative.file_name().and_then(|name| name.to_str()) else {
        return sample;
    };
    if !scope_name.starts_with("emumi-") || !scope_name.ends_with(".scope") {
        return sample;
    }
    sample.scope_name = Some(scope_name.to_owned());
    let cgroup = Path::new("/sys/fs/cgroup").join(relative);
    sample.memory_current_bytes = read_number(cgroup.join("memory.current"));
    sample.memory_high_bytes = read_limit(cgroup.join("memory.high"));
    sample.memory_max_bytes = read_limit(cgroup.join("memory.max"));
    sample.memory_swap_current_bytes = read_number(cgroup.join("memory.swap.current"));
    sample.memory_events = read_key_values(cgroup.join("memory.events"));
    let pressure = fs::read_to_string(cgroup.join("memory.pressure")).unwrap_or_default();
    sample.pressure_some_avg10 = pressure_avg10(&pressure, "some");
    sample.pressure_full_avg10 = pressure_avg10(&pressure, "full");
    sample.warning = memory_warning(&sample);
    sample
}

fn find_emulator_pid(proc_root: &Path, profile_name: &str) -> Option<u32> {
    let mut matches = fs::read_dir(proc_root)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let pid = entry.file_name().to_string_lossy().parse::<u32>().ok()?;
            let command = fs::read(entry.path().join("cmdline")).ok()?;
            let arguments = command.split(|byte| *byte == 0).collect::<Vec<_>>();
            let is_emulator = arguments.iter().any(|argument| {
                let text = String::from_utf8_lossy(argument);
                text.contains("qemu-system") || text.ends_with("/emulator")
            });
            let owns_profile = arguments
                .windows(2)
                .any(|pair| pair[0] == b"-avd" && pair[1] == profile_name.as_bytes());
            (is_emulator && owns_profile).then_some(pid)
        })
        .collect::<Vec<_>>();
    matches.sort_unstable();
    matches.into_iter().next()
}

fn process_cgroup(process: &Path) -> Option<PathBuf> {
    fs::read_to_string(process.join("cgroup"))
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("0::/"))
        .map(PathBuf::from)
}

fn status_kb(text: &str, key: &str) -> u64 {
    text.lines()
        .find_map(|line| line.strip_prefix(key))
        .and_then(|value| value.split_whitespace().next())
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

fn read_number(path: PathBuf) -> Option<u64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn read_limit(path: PathBuf) -> Option<u64> {
    let value = fs::read_to_string(path).ok()?;
    (value.trim() != "max")
        .then(|| value.trim().parse().ok())
        .flatten()
}

fn read_key_values(path: PathBuf) -> BTreeMap<String, u64> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(' ')?;
            Some((key.to_owned(), value.parse().ok()?))
        })
        .collect()
}

fn pressure_avg10(text: &str, category: &str) -> Option<f32> {
    let line = text.lines().find(|line| line.starts_with(category))?;
    line.split_whitespace()
        .find_map(|field| field.strip_prefix("avg10="))?
        .parse()
        .ok()
}

fn memory_warning(sample: &DeviceHostStats) -> Option<String> {
    let events = &sample.memory_events;
    if events.get("oom_kill").copied().unwrap_or(0) > 0
        || events.get("oom").copied().unwrap_or(0) > 0
        || events.get("max").copied().unwrap_or(0) > 0
    {
        return Some(
            "The host memory ceiling was reached; this emulator may have stalled or been killed"
                .into(),
        );
    }
    if events.get("high").copied().unwrap_or(0) > 0 {
        return Some(
            "The host is reclaiming this emulator above MemoryHigh; watch for game stutter".into(),
        );
    }
    if sample.pressure_some_avg10.unwrap_or(0.0) >= 10.0
        || sample.pressure_full_avg10.unwrap_or(0.0) >= 2.0
    {
        return Some(
            "Sustained cgroup memory pressure detected; automation latency may increase".into(),
        );
    }
    None
}

impl HostMonitor {
    pub fn sample(&mut self) -> HostStats {
        let (cpu_percent, next_cpu) = self.cpu_sample();
        self.previous_cpu = next_cpu;
        let (memory_used_bytes, memory_total_bytes, memory_available_bytes) =
            memory_sample(&self.proc_root.join("meminfo"));
        HostStats {
            cpu_percent,
            memory_used_bytes,
            memory_total_bytes,
            memory_available_bytes,
            cpu_temperature_c: cpu_temperature_c(&self.sys_root),
        }
    }

    fn cpu_sample(&self) -> (f32, Option<(u64, u64)>) {
        let Some(line) = fs::read_to_string(self.proc_root.join("stat"))
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

    #[cfg(test)]
    pub(crate) fn for_roots(proc_root: PathBuf, sys_root: PathBuf) -> Self {
        Self {
            previous_cpu: None,
            proc_root,
            sys_root,
        }
    }
}

fn memory_sample(path: &Path) -> (u64, u64, u64) {
    let Ok(text) = fs::read_to_string(path) else {
        return (0, 0, 0);
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
    (
        (total_kb - available_kb) * 1024,
        total_kb * 1024,
        available_kb * 1024,
    )
}

fn cpu_temperature_c(sys_root: &Path) -> Option<f32> {
    let mut readings = Vec::new();
    if let Ok(zones) = fs::read_dir(sys_root.join("class/thermal")) {
        for zone in zones.flatten() {
            let path = zone.path();
            let sensor_type = fs::read_to_string(path.join("type"))
                .unwrap_or_default()
                .to_ascii_lowercase();
            if (sensor_type.contains("cpu") || sensor_type.contains("x86_pkg_temp"))
                && let Some(value) = read_temperature(path.join("temp"))
            {
                readings.push(value);
            }
        }
    }
    if let Ok(devices) = fs::read_dir(sys_root.join("class/hwmon")) {
        for device in devices.flatten() {
            let path = device.path();
            let name = fs::read_to_string(path.join("name"))
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !["coretemp", "k10temp", "zenpower"]
                .iter()
                .any(|driver| name.trim() == *driver)
            {
                continue;
            }
            let Ok(entries) = fs::read_dir(path) else {
                continue;
            };
            for entry in entries.flatten() {
                let file_name = entry.file_name();
                let file_name = file_name.to_string_lossy();
                if file_name.starts_with("temp")
                    && file_name.ends_with("_input")
                    && let Some(value) = read_temperature(entry.path())
                {
                    readings.push(value);
                }
            }
        }
    }
    readings.into_iter().reduce(f32::max)
}

fn read_temperature(path: PathBuf) -> Option<f32> {
    let value: f32 = fs::read_to_string(path).ok()?.trim().parse().ok()?;
    Some(if value > 1000.0 {
        value / 1000.0
    } else {
        value
    })
}

fn parse_kb(value: &str) -> u64 {
    value
        .split_whitespace()
        .next()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_proc_and_pressure_values() {
        assert_eq!(status_kb("VmRSS:\t123 kB\nVmSwap:\t9 kB\n", "VmRSS:"), 123);
        assert_eq!(
            pressure_avg10("some avg10=12.50 avg60=2.0 total=1\n", "some"),
            Some(12.5)
        );
    }

    #[test]
    fn warns_for_high_and_hard_limit_events() {
        let mut sample = DeviceHostStats::default();
        sample.memory_events.insert("high".into(), 2);
        assert!(memory_warning(&sample).unwrap().contains("MemoryHigh"));
        sample.memory_events.insert("oom_kill".into(), 1);
        assert!(memory_warning(&sample).unwrap().contains("ceiling"));
    }

    #[test]
    fn samples_available_memory_and_millidegree_cpu_temperature() {
        let root = std::env::temp_dir().join(format!("emumi-monitor-test-{}", std::process::id()));
        let proc_root = root.join("proc");
        let sys_root = root.join("sys");
        let sensor = sys_root.join("class/hwmon/hwmon0");
        let thermal_zone = sys_root.join("class/thermal/thermal_zone0");
        fs::create_dir_all(&proc_root).unwrap();
        fs::create_dir_all(&sensor).unwrap();
        fs::create_dir_all(&thermal_zone).unwrap();
        fs::write(proc_root.join("stat"), "cpu 100 0 100 800 0 0 0 0\n").unwrap();
        fs::write(
            proc_root.join("meminfo"),
            "MemTotal: 16777216 kB\nMemAvailable: 9437184 kB\n",
        )
        .unwrap();
        fs::write(sensor.join("name"), "coretemp\n").unwrap();
        fs::write(sensor.join("temp1_input"), "77000\n").unwrap();
        fs::write(sensor.join("temp2_input"), "80000\n").unwrap();
        fs::write(thermal_zone.join("type"), "x86_pkg_temp\n").unwrap();
        fs::write(thermal_zone.join("temp"), "91000\n").unwrap();

        let mut monitor = HostMonitor::for_roots(proc_root, sys_root);
        let sample = monitor.sample();

        assert_eq!(sample.memory_total_bytes, 16 * 1024 * 1024 * 1024);
        assert_eq!(sample.memory_available_bytes, 9 * 1024 * 1024 * 1024);
        assert_eq!(sample.cpu_temperature_c, Some(91.0));
        fs::remove_dir_all(root).unwrap();
    }
}
