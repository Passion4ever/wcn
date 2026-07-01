// 纯解析/格式化函数,无 IO,输入均为字符串。行为对照 Python 版 wcn2.py。
//
// CLK_TCK / PAGE_KB:x86_64 Linux 上 USER_HZ 恒为 100、页大小恒为 4KB;
// 为避免引入 libc 依赖在此硬编码(目标平台明确)。

pub const CLK_TCK: f64 = 100.0;
pub const PAGE_KB: i64 = 4;

#[derive(Debug, Clone, PartialEq)]
pub struct Gpu {
    pub idx: String,
    pub name: String,
    pub temp: i64,
    pub util: i64,
    pub mu: i64,
    pub mt: i64,
    pub pw: i64,
    pub plim: i64,
}

/// 安全转 int:浮点字符串/[N/A] 等转不动则 0(对应 Python _i)。
pub fn pi(s: &str) -> i64 {
    s.trim().parse::<f64>().map(|x| x as i64).unwrap_or(0)
}

/// (idle, total) from /proc/stat 首行 "cpu ..."。idle = idle + iowait。
pub fn parse_proc_stat(text: &str) -> (i64, i64) {
    for line in text.lines() {
        if line.starts_with("cpu ") {
            let nums: Vec<i64> = line
                .split_whitespace()
                .skip(1)
                .filter_map(|x| x.parse().ok())
                .collect();
            if nums.len() < 4 {
                return (0, 0);
            }
            let idle = nums[3] + nums.get(4).copied().unwrap_or(0);
            let total: i64 = nums.iter().sum();
            return (idle, total);
        }
    }
    (0, 0)
}

pub fn cpu_percent(prev: (i64, i64), cur: (i64, i64)) -> f64 {
    let di = (cur.0 - prev.0) as f64;
    let dt = (cur.1 - prev.1) as f64;
    if dt <= 0.0 {
        return 0.0;
    }
    ((1.0 - di / dt) * 100.0).clamp(0.0, 100.0)
}

/// (内存用GB, 内存总GB, swap用GB, swap总GB)
pub fn parse_meminfo(text: &str) -> (f64, f64, f64, f64) {
    let get = |key: &str| -> f64 {
        for line in text.lines() {
            if let Some((k, rest)) = line.split_once(':') {
                if k == key {
                    return rest
                        .split_whitespace()
                        .next()
                        .and_then(|v| v.parse::<f64>().ok())
                        .unwrap_or(0.0);
                }
            }
        }
        0.0
    };
    let to_gb = |kb: f64| kb / 1024.0 / 1024.0;
    let total = to_gb(get("MemTotal"));
    let avail = to_gb(get("MemAvailable"));
    let sw_total = to_gb(get("SwapTotal"));
    let sw_free = to_gb(get("SwapFree"));
    (total - avail, total, sw_total - sw_free, sw_total)
}

const VIRT_IFACE: &[&str] = &[
    "docker", "veth", "br-", "virbr", "vnet", "tap", "cali", "cni", "flannel", "kube",
];

/// (rx, tx) 字节;排除 lo 及虚拟网卡前缀(只统计真实物理网卡)。
pub fn parse_net_dev(text: &str) -> (i64, i64) {
    let (mut rx, mut tx) = (0i64, 0i64);
    for line in text.lines() {
        let Some((iface, rest)) = line.split_once(':') else {
            continue;
        };
        let iface = iface.trim();
        if iface == "lo" || VIRT_IFACE.iter().any(|p| iface.starts_with(p)) {
            continue;
        }
        let f: Vec<&str> = rest.split_whitespace().collect();
        if f.len() < 9 {
            continue;
        }
        rx += f[0].parse::<i64>().unwrap_or(0);
        tx += f[8].parse::<i64>().unwrap_or(0);
    }
    (rx, tx)
}

/// 网速 (下行MB/s, 上行MB/s),两次字节计数差 / dt。
pub fn net_speed(prev: (i64, i64), cur: (i64, i64), dt: f64) -> (f64, f64) {
    if dt <= 0.0 {
        return (0.0, 0.0);
    }
    let down = ((cur.0 - prev.0) as f64 / dt / 1e6).max(0.0);
    let up = ((cur.1 - prev.1) as f64 / dt / 1e6).max(0.0);
    (down, up)
}

/// (pswpin, pswpout) 累计页计数。
pub fn parse_vmstat(text: &str) -> (i64, i64) {
    let (mut si, mut so) = (0i64, 0i64);
    for line in text.lines() {
        let p: Vec<&str> = line.split_whitespace().collect();
        if p.len() == 2 {
            match p[0] {
                "pswpin" => si = p[1].parse().unwrap_or(0),
                "pswpout" => so = p[1].parse().unwrap_or(0),
                _ => {}
            }
        }
    }
    (si, so)
}

/// 单进程瞬时 CPU%(可超 100,跨核累加)。缺前值/无间隔则 0。
pub fn proc_cpu_pct(prev: Option<i64>, cur: Option<i64>, dt: f64) -> f64 {
    match (prev, cur) {
        (Some(p), Some(c)) if dt > 0.0 => (100.0 * (c - p) as f64 / CLK_TCK / dt).max(0.0),
        _ => 0.0,
    }
}

pub fn fmt_etime(sec: i64) -> String {
    let (d, rem) = (sec / 86400, sec % 86400);
    let (h, rem) = (rem / 3600, rem % 3600);
    let (m, s) = (rem / 60, rem % 60);
    if d > 0 {
        format!("{}d{}h", d, h)
    } else if h > 0 {
        format!("{}h{}m", h, m)
    } else if m > 0 {
        format!("{}m{}s", m, s)
    } else {
        format!("{}s", s)
    }
}

pub fn fmt_rss(kb: i64) -> String {
    let mb = kb as f64 / 1024.0;
    if mb >= 1024.0 {
        format!("{:.1}G", mb / 1024.0)
    } else {
        format!("{:.0}M", mb)
    }
}

/// 命令瘦身:绝对路径取 basename、家目录折叠为 ~;内核线程 [..]/裸命令不动。
pub fn shorten_cmd(cmd: &str, home: &str) -> String {
    let mut parts: Vec<String> = cmd.split_whitespace().map(|s| s.to_string()).collect();
    if parts.is_empty() {
        return cmd.to_string();
    }
    if parts[0].starts_with('/') {
        if let Some(base) = parts[0].rsplit('/').next() {
            parts[0] = base.to_string();
        }
    }
    let out = parts.join(" ");
    if !home.is_empty() && home != "/" {
        out.replace(home, "~")
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_proc_stat_and_cpu() {
        let prev = parse_proc_stat("cpu  100 0 100 800 0 0 0 0\n");
        let cur = parse_proc_stat("cpu  200 0 200 1500 100 0 0 200\n");
        assert_eq!(prev, (800, 1000));
        assert_eq!(cur, (1600, 2200));
        assert!((cpu_percent(prev, cur) - 33.333).abs() < 0.1);
        assert_eq!(cpu_percent((800, 1000), (800, 1000)), 0.0);
    }

    #[test]
    fn t_meminfo() {
        let txt = "MemTotal: 262144000 kB\nMemFree: 1000 kB\nMemAvailable: 131072000 kB\nSwapTotal: 65536000 kB\nSwapFree: 32768000 kB\n";
        let (used, total, sw_used, sw_total) = parse_meminfo(txt);
        assert!((total - 250.0).abs() < 0.1);
        assert!((used - 125.0).abs() < 0.1);
        assert!((sw_total - 62.5).abs() < 0.1);
        assert!((sw_used - 31.25).abs() < 0.1);
    }

    #[test]
    fn t_net_dev() {
        let txt = "    lo: 999 0 0 0 0 0 0 0 888 0 0 0 0 0 0 0\n\
                   docker0: 500 0 0 0 0 0 0 0 600 0 0 0 0 0 0 0\n\
                   veth1: 700 0 0 0 0 0 0 0 800 0 0 0 0 0 0 0\n\
                   eth0: 1000 0 0 0 0 0 0 0 2000 0 0 0 0 0 0 0\n\
                   ib0: 300 0 0 0 0 0 0 0 400 0 0 0 0 0 0 0\n";
        assert_eq!(parse_net_dev(txt), (1300, 2400));
    }

    #[test]
    fn t_net_speed() {
        let (d, u) = net_speed((0, 0), (2_000_000, 1_000_000), 2.0);
        assert!((d - 1.0).abs() < 1e-6 && (u - 0.5).abs() < 1e-6);
        assert_eq!(net_speed((0, 0), (100, 100), 0.0), (0.0, 0.0));
    }

    #[test]
    fn t_vmstat() {
        assert_eq!(parse_vmstat("nr_x 1\npswpin 1000\npswpout 2000\n"), (1000, 2000));
        assert_eq!(parse_vmstat(""), (0, 0));
    }

    #[test]
    fn t_proc_cpu_pct() {
        assert_eq!(proc_cpu_pct(Some(0), Some(100), 1.0), 100.0);
        assert_eq!(proc_cpu_pct(Some(0), Some(200), 1.0), 200.0);
        assert_eq!(proc_cpu_pct(Some(100), Some(100), 1.0), 0.0);
        assert_eq!(proc_cpu_pct(Some(0), Some(100), 0.0), 0.0);
        assert_eq!(proc_cpu_pct(None, Some(5), 1.0), 0.0);
    }

    #[test]
    fn t_fmt_etime() {
        assert_eq!(fmt_etime(0), "0s");
        assert_eq!(fmt_etime(45), "45s");
        assert_eq!(fmt_etime(125), "2m5s");
        assert_eq!(fmt_etime(3725), "1h2m");
        assert_eq!(fmt_etime(90061), "1d1h");
    }

    #[test]
    fn t_fmt_rss() {
        assert_eq!(fmt_rss(0), "0M");
        assert_eq!(fmt_rss(879000), "858M");
        assert_eq!(fmt_rss(2400000), "2.3G");
    }

    #[test]
    fn t_shorten_cmd() {
        assert_eq!(
            shorten_cmd("/opt/anaconda3/bin/python /home/u/train.py", "/home/u"),
            "python ~/train.py"
        );
        assert_eq!(shorten_cmd("[kworker/5:2-events]", "/home/u"), "[kworker/5:2-events]");
        assert_eq!(shorten_cmd("python train.py", "/home/u"), "python train.py");
    }
}
