mod collect;
mod parse;
mod render;

use collect::{Sampler, Snapshot};
use ratatui::crossterm::{
    cursor::{Hide, Show},
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use render::Mode;
use std::io::{self, Stdout};
use std::panic::AssertUnwindSafe;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// 终端状态守卫:进入 raw+备用屏,Drop 时(正常退出/panic 解栈)恢复。
struct TermGuard;
impl TermGuard {
    fn new() -> io::Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, Hide)?;
        Ok(TermGuard)
    }
}
impl Drop for TermGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
    }
}

fn read_host() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn now_str() -> String {
    Command::new("date")
        .arg("+%Y-%m-%d %H:%M:%S")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn main() -> io::Result<()> {
    let me = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "?".into());
    let host = read_host();

    // CUDA 版本(慢,~0.8s)+ 最新快照,都由后台线程写、主线程读。
    // 驱动版本已折进每帧 GPU 查询,立即显示、无需慢调用。
    let cuda: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let shared: Arc<Mutex<(Option<Snapshot>, u64)>> = Arc::new(Mutex::new((None, 0)));

    // CUDA 版本:query-gpu 无此字段,只能 nvidia-smi -q(~0.8s)。独立线程拉,且
    // **等首帧出屏后再跑**——否则和首次采样的 nvidia-smi 争用驱动,拖慢进入。就绪后飘进标题。
    {
        let cuda = cuda.clone();
        let shared = shared.clone();
        std::thread::spawn(move || {
            loop {
                if shared.lock().unwrap().1 >= 1 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(30));
            }
            let v = collect::read_cuda_version();
            *cuda.lock().unwrap() = v;
        });
    }

    // 后台采样线程 → 共享 (最新快照, 版本号)
    {
        let shared = shared.clone();
        std::thread::spawn(move || {
            let mut sampler = Sampler::new(cuda);
            // 预热:基线已在 new() 建好,睡一小会儿再出首帧,让首帧就有真实
            // CPU%/网速(否则首帧两次采样间隔≈0,速率全是 0,会突兀跳一下)。
            std::thread::sleep(Duration::from_millis(300));
            loop {
                // 每帧容错:采样 panic 也不弄垮线程,保留上一帧
                if let Ok(snap) = std::panic::catch_unwind(AssertUnwindSafe(|| sampler.sample())) {
                    let mut g = shared.lock().unwrap();
                    g.0 = Some(snap);
                    g.1 += 1;
                }
                std::thread::sleep(Duration::from_millis(800));
            }
        });
    }

    let _guard = TermGuard::new()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal: Terminal<CrosstermBackend<Stdout>> = Terminal::new(backend)?;

    let mut mode = Mode::Full;
    let mut rev = false;
    let mut paused = false;
    // 当前显示的快照(暂停时冻结:不再拉新数据,连时钟一起定格)
    let mut cur: Option<(Snapshot, u64, String)> = None;
    let mut last: Option<(u64, u16, u16, bool, bool, bool)> = None;

    loop {
        // 未暂停时拉取最新快照(连同采集时刻的时钟一起存);暂停时保持冻结
        if !paused {
            let (snap_opt, ver) = {
                let g = shared.lock().unwrap();
                (g.0.clone(), g.1)
            };
            if let Some(snap) = snap_opt {
                if cur.as_ref().map(|c| c.1) != Some(ver) {
                    cur = Some((snap, ver, now_str()));
                }
            }
        }

        if let Some((snap, ver, now)) = &cur {
            let size = terminal.size()?;
            let key = (*ver, size.width, size.height, matches!(mode, Mode::Cpu), rev, paused);
            if last != Some(key) {
                last = Some(key);
                terminal.draw(|f| render::render(f, snap, mode, rev, &me, &host, now, paused))?;
            }
        }

        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press || k.kind == KeyEventKind::Repeat => {
                    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
                    match k.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Char('c') if ctrl => break, // raw 模式 Ctrl-C
                        KeyCode::Char('c') => {
                            mode = match mode {
                                Mode::Full => Mode::Cpu,
                                Mode::Cpu => Mode::Full,
                            }
                        }
                        KeyCode::Char('r') => rev = !rev,
                        KeyCode::Char('p') => paused = !paused, // 定格 / 继续
                        _ => {}
                    }
                }
                Event::Resize(_, _) => {
                    // 不清屏(清屏会闪);ratatui draw() 内部按新尺寸自动重排,只需触发一次重绘
                    last = None;
                }
                _ => {}
            }
        }
    }
    Ok(())
}
