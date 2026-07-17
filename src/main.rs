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
use std::cell::RefCell;
use std::io::{self, Write};
use std::panic::AssertUnwindSafe;
use std::process::Command;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// stdout 的阻塞/非阻塞开关。终端不排空时 write 会阻塞,主线程就会卡死在 draw 里
/// (实测能卡 13 秒:按键失灵、还积压一堆过期帧)。非阻塞 + 自己冲刷可彻底避免。
fn set_stdout_nonblocking(on: bool) {
    unsafe {
        let fl = libc::fcntl(libc::STDOUT_FILENO, libc::F_GETFL);
        if fl < 0 {
            return;
        }
        let new = if on {
            fl | libc::O_NONBLOCK
        } else {
            fl & !libc::O_NONBLOCK
        };
        libc::fcntl(libc::STDOUT_FILENO, libc::F_SETFL, new);
    }
}

/// 渲染字节先落到内存,draw() 因此永不阻塞;再由 flush_pending 非阻塞地送进 stdout。
#[derive(Clone, Default)]
struct Sink(Rc<RefCell<Vec<u8>>>);
impl Write for Sink {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// 尽量把 pending 冲进 stdout。返回是否冲完;WouldBlock 表示终端吃不下了,
/// 剩下的留到下轮再冲(绝不阻塞)。
fn flush_pending(pending: &mut Vec<u8>) -> io::Result<bool> {
    while !pending.is_empty() {
        let n = unsafe {
            libc::write(
                libc::STDOUT_FILENO,
                pending.as_ptr() as *const libc::c_void,
                pending.len(),
            )
        };
        if n > 0 {
            pending.drain(..n as usize);
            continue;
        }
        let e = io::Error::last_os_error();
        match e.kind() {
            io::ErrorKind::Interrupted => continue,
            io::ErrorKind::WouldBlock => return Ok(false),
            _ => return Err(e),
        }
    }
    Ok(true)
}

/// 终端状态守卫:进入 raw+备用屏,Drop 时(正常退出/panic 解栈)恢复。
struct TermGuard;
impl TermGuard {
    fn new() -> io::Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, Hide)?;
        set_stdout_nonblocking(true);
        Ok(TermGuard)
    }
}
impl Drop for TermGuard {
    fn drop(&mut self) {
        set_stdout_nonblocking(false); // 先恢复阻塞,否则收尾的转义序列可能写不全
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

/// --version / --help:打印后直接退出,不进 TUI。
fn handle_args() -> bool {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("wcn {}", env!("CARGO_PKG_VERSION"));
        return true;
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "wcn {} — 终端 GPU + 系统监控\n\n\
             用法:\n  \
             wcn             启动监控界面\n  \
             wcn --version   显示版本\n  \
             wcn --help      显示本帮助\n\n\
             界面按键:\n  \
             c  只看 CPU 进程   r  反序   p  定格   q  退出",
            env!("CARGO_PKG_VERSION")
        );
        return true;
    }
    false
}

fn main() -> io::Result<()> {
    if handle_args() {
        return Ok(());
    }
    let me = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "?".into());
    let host = read_host();

    // NVML 版:驱动/CUDA 版本由 NVML 瞬时读(Sampler 内一次搞定),不再需要后台版本线程。
    let shared: Arc<Mutex<(Option<Snapshot>, u64)>> = Arc::new(Mutex::new((None, 0)));

    // 后台采样线程 → 共享 (最新快照, 版本号)
    {
        let shared = shared.clone();
        std::thread::spawn(move || {
            let mut sampler = Sampler::new();
            // 预热:基线已在 new() 建好,睡一小会儿再出首帧,让首帧就有真实
            // CPU%/网速(否则首帧两次采样间隔≈0,速率全是 0,会突兀跳一下)。
            std::thread::sleep(Duration::from_millis(300));
            loop {
                let t = std::time::Instant::now();
                // 每帧容错:采样 panic 也不弄垮线程,保留上一帧
                if let Ok(snap) = std::panic::catch_unwind(AssertUnwindSafe(|| sampler.sample())) {
                    let mut g = shared.lock().unwrap();
                    g.0 = Some(snap);
                    g.1 += 1;
                }
                // 减去采样自身耗时(实测约 120ms),让整体节奏真是标题写的 0.8s,
                // 而不是 0.8s + 采样耗时 ≈ 1.0s。
                std::thread::sleep(Duration::from_millis(800).saturating_sub(t.elapsed()));
            }
        });
    }

    let _guard = TermGuard::new()?;
    let sink = Sink::default();
    let mut terminal: Terminal<CrosstermBackend<Sink>> =
        Terminal::new(CrosstermBackend::new(sink.clone()))?;
    // 已渲染但还没送进终端的字节。只有它空了才画下一帧 ——
    // 这样既不阻塞,也不会积压过期帧,ratatui 的差分状态还始终与屏幕一致。
    let mut pending: Vec<u8> = Vec::new();

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

        // 先把上一帧没送完的续上;没送完就不画新帧(终端还没吃下,画了也只是积压过期画面)
        flush_pending(&mut pending)?;

        if pending.is_empty() {
            if let Some((snap, ver, now)) = &cur {
                let size = terminal.size()?;
                let key = (*ver, size.width, size.height, matches!(mode, Mode::Cpu), rev, paused);
                if last != Some(key) {
                    last = Some(key);
                    terminal.draw(|f| render::render(f, snap, mode, rev, &me, &host, now, paused))?;
                    pending.append(&mut sink.0.borrow_mut()); // 取出这帧字节
                    flush_pending(&mut pending)?;
                }
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
