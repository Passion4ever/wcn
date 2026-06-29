// 采样:读 /proc + 调 nvidia-smi/ps,组装 Snapshot。对照 Python 版 read_* / _sampler。
use crate::parse::*;
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

pub fn read_gpus() -> Vec<Gpu> {
    let out = run(&[
        "nvidia-smi",
        "--query-gpu=index,name,temperature.gpu,utilization.gpu,\
         memory.used,memory.total,power.draw,power.limit",
        "--format=csv,noheader,nounits",
    ]);
    parse_gpu_csv(&out)
}

pub fn read_gpu_procs(
    prev: &HashMap<String, i64>,
    cur: &HashMap<String, i64>,
    dt: f64,
) -> Vec<GpuProc> {
    let rows = parse_pmon(&run(&["nvidia-smi", "pmon", "-s", "um", "-c", "1"]));
    if rows.is_empty() {
        return vec![];
    }
    let pids: Vec<String> = rows.iter().map(|r| r.pid.clone()).collect();
    let out = run(&["ps", "-o", "pid=,user=,etimes=,args=", "-p", &pids.join(",")]);
    let mut info: HashMap<String, (String, String, String)> = HashMap::new();
    for line in out.lines() {
        let f = splitn_ws(line, 4);
        if f.len() >= 4 {
            info.insert(f[0].clone(), (f[1].clone(), f[2].clone(), f[3].clone()));
        }
    }
    let mut procs: Vec<GpuProc> = rows
        .iter()
        .map(|r| {
            let (user, et, cmd) = info
                .get(&r.pid)
                .cloned()
                .unwrap_or_else(|| ("?".into(), "0".into(), "-".into()));
            let pcpu = proc_cpu_pct(prev.get(&r.pid).copied(), cur.get(&r.pid).copied(), dt);
            GpuProc {
                gpu: r.gpu.clone(),
                pid: r.pid.clone(),
                user,
                sm: r.sm.clone(),
                fb: r.fb.clone(),
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
}

impl Sampler {
    pub fn new() -> Self {
        Sampler {
            prev_stat: parse_proc_stat(&read_file("/proc/stat")),
            prev_net: parse_net_dev(&read_file("/proc/net/dev")),
            prev_vm: parse_vmstat(&read_file("/proc/vmstat")),
            prev_j: read_proc_jiffies(),
            t_prev: Instant::now(),
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
        let gpus = read_gpus();
        let gpu_procs = read_gpu_procs(&self.prev_j, &cur_j, dt);
        let cpu_procs = read_cpu_procs(&self.prev_j, &cur_j, dt, 50);
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
