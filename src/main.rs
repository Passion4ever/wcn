#![allow(dead_code)] // 各模块逐步接入,开发期允许未用

mod collect;
mod parse;
mod render;

fn main() {
    // 临时冒烟:采两次(隔 1s)打印 Snapshot 概要,验证采集正确。
    let mut s = collect::Sampler::new();
    std::thread::sleep(std::time::Duration::from_secs(1));
    let snap = s.sample();
    println!(
        "gpus={} gpu_procs={} cpu_procs={} cpu={:.0}% mem={:.0}/{:.0}G swap={:.0}/{:.0}G active={} net=↓{:.1} ↑{:.1}",
        snap.gpus.len(), snap.gpu_procs.len(), snap.cpu_procs.len(),
        snap.cpu, snap.mem.0, snap.mem.1, snap.swap.0, snap.swap.1, snap.swap.2,
        snap.net.0, snap.net.1
    );
    println!("-- top5 cpu --");
    for p in snap.cpu_procs.iter().take(5) {
        println!("  pid={:>7} cpu={:>4}% rss={} swap={} {} {}", p.pid, p.pcpu,
                 parse::fmt_rss(p.rss), parse::fmt_rss(p.swap), p.etime,
                 parse::shorten_cmd(&p.cmd, "/home/user01"));
    }
    println!("-- gpu procs --");
    for p in &snap.gpu_procs {
        println!("  gpu{} pid={:>7} {} sm={}% cpu={}% {}", p.gpu, p.pid, p.user, p.sm, p.pcpu,
                 parse::shorten_cmd(&p.cmd, "/home/user01"));
    }
}
