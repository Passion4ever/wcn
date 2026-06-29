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

    // 后台采样线程 → 共享 (最新快照, 版本号)
    let shared: Arc<Mutex<(Option<Snapshot>, u64)>> = Arc::new(Mutex::new((None, 0)));
    {
        let shared = shared.clone();
        std::thread::spawn(move || {
            let mut sampler = Sampler::new();
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
    let mut last: Option<(u64, u16, u16, bool, bool)> = None;

    loop {
        // 正常只画差异(不清屏,故不闪);花屏的根源是尺寸变化,由下方 Resize 清屏处理。
        let (snap_opt, ver) = {
            let g = shared.lock().unwrap();
            (g.0.clone(), g.1)
        };
        if let Some(snap) = &snap_opt {
            let size = terminal.size()?;
            let key = (ver, size.width, size.height, matches!(mode, Mode::Cpu), rev);
            if last != Some(key) {
                last = Some(key);
                let now = now_str();
                terminal.draw(|f| render::render(f, snap, mode, rev, &me, &host, &now))?;
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
                        _ => {}
                    }
                }
                Event::Resize(_, _) => {
                    // 尺寸变化(含改字号):清屏强制全量重绘,避免残留花屏
                    terminal.clear()?;
                    last = None;
                }
                _ => {}
            }
        }
    }
    Ok(())
}
