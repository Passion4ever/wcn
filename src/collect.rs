// 采样:读 /proc + 调 nvidia-smi/ps,组装 Snapshot。对照 Python 版 read_* / _sampler。
use crate::parse::*;
use nvml_wrapper::Nvml;
use nvml_wrapper::enum_wrappers::device::TemperatureSensor;
use nvml_wrapper::enums::device::UsedGpuMemory;
use std::collections::HashMap;
use std::fs;
use std::process::Command;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct GpuProc {
    pub gpu: String,
    pub pid: String,
    pub user: String,
    pub sm: String,
    pub fb: String,
    pub pcpu: String,
    pub etime: String,
    pub cmd: String,
}

#[derive(Debug, Clone)]
pub struct CpuProc {
    pub pid: String,
    pub user: String,
    pub pcpu: String,
    pub pmem: String,
    pub rss: i64,
    pub swap: i64,
    pub etime: String,
    pub cmd: String,
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub gpus: Vec<Gpu>,
    pub gpu_procs: Vec<GpuProc>,
    pub cpu_procs: Vec<CpuProc>,
    pub cpu: f64,
    pub mem: (f64, f64),
    pub swap: (f64, f64, bool),
    pub net: (f64, f64),
    pub uptime: f64,
    pub load: (f64, f64, f64),
    pub driver: String,
    pub cuda: String,
}

fn run(args: &[&str]) -> String {
    Command::new(args[0])
        .args(&args[1..])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

fn read_file(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

/// 从 NVML 读驱动 + CUDA 版本(进程内调用,μs 级,瞬时)。
/// CUDA 原始值形如 12080 → "12.8"(major = /1000,minor = (%1000)/10)。
pub fn read_driver_cuda(nvml: &Nvml) -> (String, String) {
    let driver = nvml.sys_driver_version().unwrap_or_default();
    let cuda = nvml
        .sys_cuda_driver_version()
        .map(|v| format!("{}.{}", v / 1000, (v % 1000) / 10))
        .unwrap_or_default();
    (driver, cuda)
}

/// 规整 GPU 名称:去 "NVIDIA " 前缀与 " PCIe" 后缀(与旧 CSV 解析行为一致)。
fn clean_name(name: &str) -> String {
    name.replace("NVIDIA ", "")
        .replace(" PCIe", "")
        .trim()
        .to_string()
}

/// 按空白把一行切成至多 n 段,最后一段保留余下(含空格),模拟 Python str.split(None, n-1)。
fn splitn_ws(line: &str, n: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = line.trim_start();
    while out.len() + 1 < n {
        match rest.find(char::is_whitespace) {
            Some(i) => {
                out.push(rest[..i].to_string());
                rest = rest[i..].trim_start();
            }
            None => break,
        }
    }
    if !rest.is_empty() {
        out.push(rest.to_string());
    }
    out
}

pub fn read_proc_jiffies() -> HashMap<String, i64> {
    let mut m = HashMap::new();
    if let Ok(rd) = fs::read_dir("/proc") {
        for e in rd.flatten() {
            let name = e.file_name();
            let pid = name.to_string_lossy().into_owned();
            if pid.is_empty() || !pid.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            if let Ok(data) = fs::read_to_string(format!("/proc/{}/stat", pid)) {
                if let Some(idx) = data.rfind(')') {
                    let rest: Vec<&str> = data[idx + 2..].split_whitespace().collect();
                    if rest.len() > 12 {
                        if let (Ok(u), Ok(s)) =
                            (rest[11].parse::<i64>(), rest[12].parse::<i64>())
                        {
                            m.insert(pid, u + s);
                        }
                    }
                }
            }
        }
    }
    m
}

pub fn read_proc_swap(pid: &str) -> i64 {
    if let Ok(data) = fs::read_to_string(format!("/proc/{}/status", pid)) {
        for line in data.lines() {
            if let Some(rest) = line.strip_prefix("VmSwap:") {
                return pi(rest.split_whitespace().next().unwrap_or("0"));
            }
        }
    }
    0
}

/// 从 NVML 读每张卡的概览(温度/利用率/显存/功耗)。全程进程内调用,μs~ms 级,不起子进程。
pub fn read_gpus(nvml: &Nvml) -> Vec<Gpu> {
    let n = nvml.device_count().unwrap_or(0);
    let mut out = Vec::new();
    for i in 0..n {
        let Ok(dev) = nvml.device_by_index(i) else {
            continue;
        };
        let name = dev.name().map(|s| clean_name(&s)).unwrap_or_default();
        let temp = dev.temperature(TemperatureSensor::Gpu).unwrap_or(0) as i64;
        let util = dev.utilization_rates().map(|u| u.gpu).unwrap_or(0) as i64;
        let (mu, mt) = dev
            .memory_info()
            .map(|m| ((m.used / 1024 / 1024) as i64, (m.total / 1024 / 1024) as i64))
            .unwrap_or((0, 0));
        let pw = dev.power_usage().unwrap_or(0) as i64 / 1000; // mW → W
        let plim = dev.enforced_power_limit().unwrap_or(0) as i64 / 1000;
        out.push(Gpu {
            idx: i.to_string(),
            name,
            temp,
            util,
            mu,
            mt,
            pw,
            plim,
        });
    }
    out
}

/// 从 NVML 读 GPU 计算进程(按卡):每进程显存 + SM% 来自 NVML;
/// user/etime/cmd 仍走一次 ps,CPU% 走 /proc 增量(与旧版一致)。
pub fn read_gpu_procs(
    nvml: &Nvml,
    prev: &HashMap<String, i64>,
    cur: &HashMap<String, i64>,
    dt: f64,
) -> Vec<GpuProc> {
    let n = nvml.device_count().unwrap_or(0);
    // (gpu_idx, pid, fb_MB, sm)
    let mut raw: Vec<(String, u32, i64, String)> = Vec::new();
    for i in 0..n {
        let Ok(dev) = nvml.device_by_index(i) else {
            continue;
        };
        // 每进程 SM%:近窗口样本,同 pid 取最新时间戳那条;无样本/报错则留空。
        let mut sm_of: HashMap<u32, (u64, u32)> = HashMap::new();
        if let Ok(samples) = dev.process_utilization_stats(None) {
            for s in samples {
                let e = sm_of.entry(s.pid).or_insert((0, 0));
                if s.timestamp >= e.0 {
                    *e = (s.timestamp, s.sm_util);
                }
            }
        }
        if let Ok(procs) = dev.running_compute_processes() {
            for p in procs {
                let fb = match p.used_gpu_memory {
                    UsedGpuMemory::Used(b) => (b / 1024 / 1024) as i64,
                    UsedGpuMemory::Unavailable => 0,
                };
                let sm = sm_of
                    .get(&p.pid)
                    .map(|(_, u)| u.to_string())
                    .unwrap_or_else(|| "-".into());
                raw.push((i.to_string(), p.pid, fb, sm));
            }
        }
    }
    if raw.is_empty() {
        return vec![];
    }
    let pids: Vec<String> = raw.iter().map(|(_, pid, _, _)| pid.to_string()).collect();
    let out = run(&["ps", "-o", "pid=,user=,etimes=,args=", "-p", &pids.join(",")]);
    let mut info: HashMap<String, (String, String, String)> = HashMap::new();
    for line in out.lines() {
        let f = splitn_ws(line, 4);
        if f.len() >= 4 {
            info.insert(f[0].clone(), (f[1].clone(), f[2].clone(), f[3].clone()));
        }
    }
    let mut procs: Vec<GpuProc> = raw
        .iter()
        .map(|(gpu, pid, fb, sm)| {
            let pid = pid.to_string();
            let (user, et, cmd) = info
                .get(&pid)
                .cloned()
                .unwrap_or_else(|| ("?".into(), "0".into(), "-".into()));
            let pcpu = proc_cpu_pct(prev.get(&pid).copied(), cur.get(&pid).copied(), dt);
            GpuProc {
                gpu: gpu.clone(),
                pid,
                user,
                sm: sm.clone(),
                fb: fb.to_string(),
                pcpu: format!("{:.0}", pcpu),
                etime: fmt_etime(pi(&et)),
                cmd,
            }
        })
        .collect();
    // 按卡分组:卡号升序;卡内按 PID 稳定排序
    procs.sort_by_key(|p| (pi(&p.gpu), pi(&p.pid)));
    procs
}

pub fn read_cpu_procs(
    prev: &HashMap<String, i64>,
    cur: &HashMap<String, i64>,
    dt: f64,
    n: usize,
) -> Vec<CpuProc> {
    let out = run(&["ps", "-eo", "pid=,user:20=,pmem=,rss=,etimes=,args="]);
    let mut procs: Vec<(f64, CpuProc)> = Vec::new();
    for line in out.lines() {
        let f = splitn_ws(line, 6);
        if f.len() < 6 {
            continue;
        }
        let pcpu = proc_cpu_pct(prev.get(&f[0]).copied(), cur.get(&f[0]).copied(), dt);
        procs.push((
            pcpu,
            CpuProc {
                pid: f[0].clone(),
                user: f[1].clone(),
                pcpu: String::new(),
                pmem: f[2].clone(),
                rss: pi(&f[3]),
                swap: 0,
                etime: fmt_etime(pi(&f[4])),
                cmd: f[5].clone(),
            },
        ));
    }
    procs.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    procs.truncate(n);
    procs
        .into_iter()
        .map(|(pc, mut p)| {
            p.pcpu = format!("{:.0}", pc);
            p.swap = read_proc_swap(&p.pid); // 仅给入选前 n 个读 VmSwap(便宜)
            p
        })
        .collect()
}

pub struct Sampler {
    prev_stat: (i64, i64),
    prev_net: (i64, i64),
    prev_vm: (i64, i64),
    prev_j: HashMap<String, i64>,
    t_prev: Instant,
    // NVML 句柄:启动 dlopen 一次;无 GPU/无驱动库则为 None(GPU 面板留空,系统面板照常)。
    // 驱动/CUDA 版本瞬时可读,启动查一次即可,无需后台线程。
    nvml: Option<Nvml>,
    driver: String,
    cuda: String,
}

impl Sampler {
    pub fn new() -> Self {
        let nvml = Nvml::init().ok();
        let (driver, cuda) = match &nvml {
            Some(n) => read_driver_cuda(n),
            None => (String::new(), String::new()),
        };
        Sampler {
            prev_stat: parse_proc_stat(&read_file("/proc/stat")),
            prev_net: parse_net_dev(&read_file("/proc/net/dev")),
            prev_vm: parse_vmstat(&read_file("/proc/vmstat")),
            prev_j: read_proc_jiffies(),
            t_prev: Instant::now(),
            nvml,
            driver,
            cuda,
        }
    }

    pub fn sample(&mut self) -> Snapshot {
        let cur_stat = parse_proc_stat(&read_file("/proc/stat"));
        let cur_net = parse_net_dev(&read_file("/proc/net/dev"));
        let (mu, mt, sw_used, sw_total) = parse_meminfo(&read_file("/proc/meminfo"));
        let cur_vm = parse_vmstat(&read_file("/proc/vmstat"));
        let cur_j = read_proc_jiffies();
        let now = Instant::now();
        let dt = now.duration_since(self.t_prev).as_secs_f64();
        let cpu = cpu_percent(self.prev_stat, cur_stat);
        let net = net_speed(self.prev_net, cur_net, dt);
        // 换页速率 MB/s:(pswpin+pswpout 增量)× 页大小;>0.5 视为正在换页
        let sw_io = if dt > 0.0 {
            (cur_vm.0 - self.prev_vm.0 + cur_vm.1 - self.prev_vm.1) as f64 * PAGE_KB as f64
                / 1024.0
                / dt
        } else {
            0.0
        };
        let (gpus, gpu_procs) = match &self.nvml {
            Some(n) => (read_gpus(n), read_gpu_procs(n, &self.prev_j, &cur_j, dt)),
            None => (vec![], vec![]),
        };
        let cpu_procs = read_cpu_procs(&self.prev_j, &cur_j, dt, 50);
        let uptime = parse_uptime(&read_file("/proc/uptime"));
        let load = parse_loadavg(&read_file("/proc/loadavg"));
        self.prev_stat = cur_stat;
        self.prev_net = cur_net;
        self.prev_vm = cur_vm;
        self.prev_j = cur_j;
        self.t_prev = now;
        Snapshot {
            gpus,
            gpu_procs,
            cpu_procs,
            cpu,
            mem: (mu, mt),
            swap: (sw_used, sw_total, sw_io > 0.5),
            net,
            uptime,
            load,
            driver: self.driver.clone(),
            cuda: self.cuda.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_splitn_ws() {
        assert_eq!(
            splitn_ws("  897746 user02 21060 python train.py --a", 4),
            vec!["897746", "user02", "21060", "python train.py --a"]
        );
        assert_eq!(splitn_ws("a b", 6), vec!["a", "b"]);
    }
}
