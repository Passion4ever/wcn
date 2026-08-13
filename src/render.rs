// ratatui 渲染。逐项对照 Python wcn2.py 的 render()。
// 设计:表格全部手绘(ratatui 的 Table 无法加表头横线/对齐表头)。
//  - 系统条:rich ProgressBar 风格的重线 ━ + 端帽 ╸(line_bar)
//  - GPU 概览显存条:实心块 █/░(cell_bar)
//  - GPU 概览 / CPU 进程:SIMPLE_HEAVY = 表头 + 全宽 ━ 横线 + 数据行(无竖线)
//  - GPU 进程:SQUARE 全网格(竖线 │ + ┼ 交叉 + 卡间横线)
// 配色相对主题:不设 fg=终端默认;dim/reversed/语义色。
use crate::collect::{CpuProc, Snapshot};
use ratatui::layout::{Alignment, Constraint, Direction, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Full,
    Cpu,
}

/// 进程筛选。User 用于 [u](只看某人,精确匹配用户名);
/// Text 用于 [/](子串匹配 用户名 或 命令,不分大小写)。
#[derive(Clone, PartialEq)]
pub enum Filter {
    User(String),
    Text(String),
}

impl Filter {
    fn hit(&self, user: &str, cmd: &str) -> bool {
        match self {
            Filter::User(u) => user == u,
            Filter::Text(q) => {
                let q = q.to_lowercase();
                user.to_lowercase().contains(&q) || cmd.to_lowercase().contains(&q)
            }
        }
    }
    /// 标题上的后缀,如 " · 仅 user01"
    fn tag(&self) -> String {
        match self {
            Filter::User(u) => format!(" · 仅 {}", u),
            Filter::Text(q) => format!(" · 匹配 \"{}\"", q),
        }
    }
}

fn keep(filter: Option<&Filter>, p: &CpuProc) -> bool {
    filter.is_none_or(|f| f.hit(&p.user, &p.cmd))
}

fn dim() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}
fn hdr_style() -> Style {
    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
}

fn pct_style(v: f64) -> Style {
    if v <= 0.0 {
        dim()
    } else if v < 50.0 {
        Style::default().fg(Color::Green)
    } else if v < 85.0 {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Red)
    }
}
fn temp_style(v: i64) -> Style {
    if v < 60 {
        Style::default().fg(Color::Green)
    } else if v < 80 {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Red)
    }
}
/// 负载按「负载/核数」上色 —— 于是红色在任何机器上都等于"超过本机核数、在排队",
/// 不用心算除核数。绿<70%,黄 70~100%,红 ≥100%。
fn load_style(load: f64, ncpu: usize) -> Style {
    let r = load / ncpu.max(1) as f64;
    if r < 0.7 {
        Style::default().fg(Color::Green)
    } else if r < 1.0 {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Red)
    }
}
fn pi(s: &str) -> f64 {
    s.trim().parse::<f64>().unwrap_or(-1.0)
}

fn panel(title: &str, color: Color) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(color))
        .title(Span::raw(format!(" {} ", title)))
}

/// 字符显示宽度(粗略东亚宽=2)。
fn disp_w(c: char) -> usize {
    let u = c as u32;
    if (0x1100..=0x115F).contains(&u)
        || (0x2E80..=0xA4CF).contains(&u)
        || (0xAC00..=0xD7A3).contains(&u)
        || (0xF900..=0xFAFF).contains(&u)
        || (0xFE30..=0xFE4F).contains(&u)
        || (0xFF00..=0xFF60).contains(&u)
        || (0xFFE0..=0xFFE6).contains(&u)
        || (0x20000..=0x3FFFD).contains(&u)
    {
        2
    } else {
        1
    }
}
fn str_w(s: &str) -> usize {
    s.chars().map(disp_w).sum()
}

/// 超显示宽截断,结尾 …。
fn truncate_w(s: &str, max: usize) -> String {
    if str_w(s) <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut w = 0;
    for ch in s.chars() {
        let cw = disp_w(ch);
        if w + cw > max - 1 {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    out
}

/// 按显示宽对齐补空格(超长则截断)。
fn pad(s: &str, w: usize, align: Alignment) -> String {
    let dw = str_w(s);
    if dw > w {
        return truncate_w(s, w);
    }
    let sp = w - dw;
    match align {
        Alignment::Right => format!("{}{}", " ".repeat(sp), s),
        Alignment::Center => {
            let l = sp / 2;
            format!("{}{}{}", " ".repeat(l), s, " ".repeat(sp - l))
        }
        _ => format!("{}{}", s, " ".repeat(sp)),
    }
}

/// 实心块条 █/░(GPU 显存列)。
fn cell_bar(pct: f64, width: usize) -> Vec<Span<'static>> {
    let pct = pct.clamp(0.0, 100.0);
    let fill = ((pct / 100.0 * width as f64).round() as usize).min(width);
    vec![
        Span::styled("█".repeat(fill), pct_style(pct)),
        Span::styled("░".repeat(width - fill), dim()),
    ]
}

/// 重线条 ━ + 端帽 ╸(rich ProgressBar 风格,系统面板用)。
fn line_bar(pct: f64, width: usize) -> Vec<Span<'static>> {
    if width == 0 {
        return vec![];
    }
    let pct = pct.clamp(0.0, 100.0);
    let n = ((pct / 100.0) * width as f64).floor() as usize;
    let n = n.min(width);
    let st = pct_style(pct);
    let mut spans = Vec::new();
    if n == 0 {
        spans.push(Span::styled("━".repeat(width), dim()));
    } else {
        spans.push(Span::styled("━".repeat(n - 1), st));
        spans.push(Span::styled("╸".to_string(), st));
        if width > n {
            spans.push(Span::styled("━".repeat(width - n), dim()));
        }
    }
    spans
}

fn hrule(w: usize) -> Line<'static> {
    Line::from(Span::raw("━".repeat(w)))
}

fn me_style(user: &str, me: &str) -> Option<Style> {
    if user == me {
        Some(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments)]
pub fn render(f: &mut Frame, snap: &Snapshot, mode: Mode, rev: bool, me: &str, host: &str, now: &str, paused: bool,
              filter: Option<&Filter>, typing: Option<&str>) {
    let area = f.area();
    let w = area.width as usize;
    let h = area.height as usize;
    let ngpu = snap.gpus.len();

    // 最小尺寸兜底
    let min_h = 12 + ngpu;
    if w < 60 || h < min_h {
        let msg = vec![
            Line::from(Span::styled("窗口太小", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(format!("当前  {} × {}", w, h)),
            Line::from(format!("建议至少  60 × {}", min_h)),
            Line::from(""),
            Line::from(Span::styled("(调小字体可获得更多空间)", dim())),
        ];
        let bw = 36u16.min(area.width);
        let bh = 8u16.min(area.height);
        let rect = Rect {
            x: area.x + area.width.saturating_sub(bw) / 2,
            y: area.y + area.height.saturating_sub(bh) / 2,
            width: bw,
            height: bh,
        };
        f.render_widget(Paragraph::new(msg).alignment(Alignment::Center).block(panel("", Color::Red)), rect);
        return;
    }

    if mode == Mode::Cpu {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)])
            .split(area);
        render_header(f, chunks[0], snap, host, now, paused);
        let n = h.saturating_sub(6).max(3);
        // 筛选在完整进程集合上做(采样端已不再截断),否则会静默漏掉大量进程
        let mut procs: Vec<&CpuProc> = snap.cpu_procs.iter().filter(|p| keep(filter, p)).collect();
        if rev {
            procs.reverse();
        }
        let shown = procs.len().min(n);
        let order = if rev { "升序" } else { "降序" };
        let tag = filter.map(|f| f.tag()).unwrap_or_default();
        let title = format!("CPU 进程{} · 共 {},显示 {}({})", tag, procs.len(), shown, order);
        let inner_w = chunks[1].width.saturating_sub(2) as usize;
        let lines = cpu_lines(&procs[..shown], me, true, inner_w);
        let bc = if filter.is_some() { Color::Yellow } else { Color::Blue };
        f.render_widget(Paragraph::new(lines).block(panel(&title, bc)), chunks[1]);
        f.render_widget(Paragraph::new(hint_line(paused, typing, true)), chunks[2]);
        return;
    }

    // 完整视图高度预算
    let sys_rows = 3 + if snap.swap.1 > 0.0 { 1 } else { 0 };
    let sys_panel_h = sys_rows + 2;
    let gpu_overview_h = ngpu + 4; // header + ━ + 各卡 + 边框2
    let side_by_side = w >= 120;
    let top_h = if side_by_side {
        sys_panel_h.max(gpu_overview_h)
    } else {
        sys_panel_h + gpu_overview_h
    };

    let ncards = {
        let mut s = std::collections::BTreeSet::new();
        for p in &snap.gpu_procs {
            s.insert(p.gpu.clone());
        }
        s.len()
    };
    let gpu_procs_h = 4 + snap.gpu_procs.len() + ncards.saturating_sub(1); // 边框2+表头1+横线1+行+卡间线

    let cprocs: Vec<&CpuProc> = snap.cpu_procs.iter().filter(|p| keep(filter, p)).collect();
    let remaining = h.saturating_sub(2 + top_h);
    let cpu_chrome = 4; // 边框2 + 表头1 + 横线1
    // .max(1):筛出 0 条时也保留面板(否则整块凭空消失,像是坏了)
    let want_cpu = cprocs.len().min(10).max(1);
    let cpu_rows = remaining.saturating_sub(gpu_procs_h + cpu_chrome).min(want_cpu);
    let cpu_panel_h = if cpu_rows >= 1 { cpu_rows + cpu_chrome } else { 0 };

    let mut cons = vec![
        Constraint::Length(1),
        Constraint::Length(top_h as u16),
        Constraint::Length(gpu_procs_h as u16),
    ];
    if cpu_panel_h > 0 {
        cons.push(Constraint::Length(cpu_panel_h as u16));
    }
    cons.push(Constraint::Min(0));
    cons.push(Constraint::Length(1));
    let chunks = Layout::default().direction(Direction::Vertical).constraints(cons).split(area);

    render_header(f, chunks[0], snap, host, now, paused);
    render_top(f, chunks[1], snap, side_by_side);
    render_gpu_procs(f, chunks[2], snap, me);
    if cpu_panel_h > 0 {
        let procs: Vec<&CpuProc> = cprocs.iter().take(cpu_rows).copied().collect();
        let inner_w = chunks[3].width.saturating_sub(2) as usize;
        let lines = cpu_lines(&procs, me, false, inner_w);
        let tag = filter.map(|f| f.tag()).unwrap_or_default();
        let bc = if filter.is_some() { Color::Yellow } else { Color::Blue };
        f.render_widget(Paragraph::new(lines).block(panel(&format!("CPU 进程 Top 10{}", tag), bc)), chunks[3]);
    }
    let hint = chunks.last().unwrap();
    f.render_widget(Paragraph::new(hint_line(paused, typing, false)), *hint);
}

/// 底部提示行。输入搜索时整行让位给搜索框。
fn hint_line<'a>(paused: bool, typing: Option<&str>, cpu_page: bool) -> Line<'a> {
    if let Some(buf) = typing {
        return Line::from(vec![
            Span::raw("  "),
            Span::styled("搜索: ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(buf.to_string(), Style::default().fg(Color::Yellow)),
            Span::styled("▏", Style::default().fg(Color::Yellow)),
            Span::styled("   (用户名或命令   回车确认   Esc 取消)", dim()),
        ]);
    }
    let pk = if paused { "[p] 继续" } else { "[p] 暂停" };
    // 标签收紧到最小宽度 60 列也放得下(加了 u / 搜索两项后,原来的写法会被截掉"退出")
    let head = if cpu_page { "  [c] 返回  [r] 反序" } else { "  [c] CPU页" };
    Line::from(Span::styled(
        format!("{}  [u] 仅我  [/] 搜索  {}  [q] 退出", head, pk),
        dim(),
    ))
}

fn render_header(f: &mut Frame, area: Rect, snap: &Snapshot, host: &str, now: &str, paused: bool) {
    // 左:身份类信息(版本 / 主机 / 开机时长)
    let left = Line::from(vec![
        Span::styled(concat!("wcn v", env!("CARGO_PKG_VERSION")), hdr_style()),
        Span::styled(format!(" @ {}", host), dim()),
        Span::styled(
            format!(" · 开机 {}", crate::parse::fmt_uptime(snap.uptime as i64)),
            dim(),
        ),
    ]);
    let tail = if paused {
        Span::styled("⏸ 已暂停", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
    } else {
        Span::styled("刷新 0.8s", dim())
    };
    // 右:动态信息(负载 1/5/15 分钟 / 时间 / 刷新)。窄了先裁掉最左的负载,时间/刷新恒可见。
    // 负载三个数各按「值/核数」上色,红=超过本机核数(跨机器统一,不用心算)。
    let ncpu = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let (l1, l5, l15) = snap.load;
    let mut spans = vec![Span::styled("负载 ", dim())];
    for (i, v) in [l1, l5, l15].iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" ", dim()));
        }
        spans.push(Span::styled(format!("{:.2}", v), load_style(*v, ncpu)));
    }
    spans.push(Span::styled("  ·  ", dim()));
    spans.push(Span::styled(format!("{}  ·  ", now), dim()));
    spans.push(tail);
    let right = Line::from(spans).alignment(Alignment::Right);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);
    f.render_widget(Paragraph::new(left), cols[0]);
    f.render_widget(Paragraph::new(right), cols[1]);
}

fn render_top(f: &mut Frame, area: Rect, snap: &Snapshot, side_by_side: bool) {
    if side_by_side {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(34), Constraint::Min(0)])
            .split(area);
        render_sys(f, cols[0], snap);
        render_gpu_overview(f, cols[1], snap);
    } else {
        let sys_rows = 3 + if snap.swap.1 > 0.0 { 1 } else { 0 };
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length((sys_rows + 2) as u16), Constraint::Min(0)])
            .split(area);
        render_sys(f, rows[0], snap);
        render_gpu_overview(f, rows[1], snap);
    }
}

enum Mid {
    Bar(f64),
    Text(Vec<Span<'static>>),
}

fn render_sys(f: &mut Frame, area: Rect, snap: &Snapshot) {
    let block = panel("系统", Color::Cyan);
    let inner0 = block.inner(area);
    f.render_widget(block, area);
    // 左右各留 1 空格(对齐 Python panel padding)
    let inner = Rect {
        x: inner0.x + 1,
        y: inner0.y,
        width: inner0.width.saturating_sub(2),
        height: inner0.height,
    };
    let (mu, mt) = snap.mem;
    let mem_pct = if mt > 0.0 { mu / mt * 100.0 } else { 0.0 };
    let cpu = snap.cpu;
    let (sw_used, sw_total, sw_active) = snap.swap;
    let (down, up) = snap.net;

    let mut rows: Vec<(Span, Mid, Line)> = Vec::new();
    rows.push((
        Span::styled("CPU", Style::default().fg(Color::Cyan)),
        Mid::Bar(cpu),
        Line::from(Span::styled(format!("{:.0}%", cpu), pct_style(cpu))).alignment(Alignment::Right),
    ));
    rows.push((
        Span::styled("内存", Style::default().fg(Color::Magenta)),
        Mid::Bar(mem_pct),
        Line::from(Span::styled(format!("{:.0}/{:.0}G", mu, mt), pct_style(mem_pct))).alignment(Alignment::Right),
    ));
    if sw_total > 0.0 {
        let sw_pct = sw_used / sw_total * 100.0;
        let (col, tail) = if sw_active {
            (Color::Red, " ⇅")
        } else if sw_used > 0.0 {
            (Color::Yellow, "")
        } else {
            (Color::Gray, "")
        };
        rows.push((
            Span::styled("交换", Style::default().fg(col)),
            Mid::Bar(sw_pct),
            Line::from(Span::styled(format!("{:.0}/{:.0}G{}", sw_used, sw_total, tail), Style::default().fg(col)))
                .alignment(Alignment::Right),
        ));
    }
    rows.push((
        Span::styled("网络", Style::default().fg(Color::Blue)),
        Mid::Text(vec![
            Span::styled(format!("↓ {:.1}  ", down), Style::default().fg(Color::Green)),
            Span::styled(format!("↑ {:.1}", up), Style::default().fg(Color::Yellow)),
        ]),
        Line::from(Span::styled("MB/s", dim())).alignment(Alignment::Right),
    ));

    let row_areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Length(1); rows.len()])
        .flex(Flex::Center)
        .split(inner);
    for (i, (label, mid, value)) in rows.into_iter().enumerate() {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(5), Constraint::Min(4), Constraint::Length(10)])
            .split(row_areas[i]);
        f.render_widget(Paragraph::new(Line::from(label)), cols[0]);
        match mid {
            Mid::Bar(r) => {
                let bar = line_bar(r, cols[1].width as usize);
                f.render_widget(Paragraph::new(Line::from(bar)), cols[1]);
            }
            Mid::Text(spans) => {
                f.render_widget(Paragraph::new(Line::from(spans)), cols[1]);
            }
        }
        f.render_widget(Paragraph::new(value), cols[2]);
    }
}

fn render_gpu_overview(f: &mut Frame, area: Rect, snap: &Snapshot) {
    let title = if snap.driver.is_empty() {
        "GPU".to_string()
    } else if snap.cuda.is_empty() {
        format!("GPU · 驱动 {}", snap.driver)
    } else {
        format!("GPU · 驱动 {} · CUDA {}", snap.driver, snap.cuda)
    };
    let block = panel(&title, Color::Green);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let iw = inner.width as usize;
    if snap.gpus.is_empty() {
        f.render_widget(Paragraph::new(" 无 NVIDIA GPU / nvidia-smi 不可用"), inner);
        return;
    }
    let show_membar = iw >= 80;
    let headers = ["卡号", "名称", "显存用量", "利用率", "温度", "功耗"];
    let aligns = [
        Alignment::Center,
        Alignment::Left,
        Alignment::Left,
        Alignment::Right,
        Alignment::Right,
        Alignment::Right,
    ];
    // 各 GPU 的文本(用于算列宽)
    // 四舍五入而非截断:截断会把 1892MiB 说成 "1G"(实为 1.85G),
    // 也会把 A6000 的 49140MiB 总量说成 "47G"(实为 48G)。
    let gb = |m: i64| (m as f64 / 1024.0).round() as i64;
    let gb_of = |g: &crate::parse::Gpu| format!("{}/{}G", gb(g.mu), gb(g.mt));
    let max_gb = snap.gpus.iter().map(|g| str_w(&gb_of(g))).max().unwrap_or(5);
    let vram_content = if show_membar { 11 + max_gb } else { max_gb }; // 块条10+空格1+GB
    // 列内容宽 = max(表头, 各行)
    let mut w = [0usize; 6];
    for (i, hdr) in headers.iter().enumerate() {
        w[i] = str_w(hdr);
    }
    for g in &snap.gpus {
        w[0] = w[0].max(str_w(&g.idx));
        w[1] = w[1].max(str_w(&g.name));
        w[2] = w[2].max(vram_content);
        w[3] = w[3].max(str_w(&format!("{}%", g.util)));
        w[4] = w[4].max(str_w(&format!("{}°C", g.temp)));
        w[5] = w[5].max(str_w(&format!("{}/{}W", g.pw, g.plim)));
    }
    // 剩余宽度做成列间等间距(左右各留 1)
    let content_sum: usize = w.iter().sum();
    let avail = iw.saturating_sub(content_sum + 2);
    let ngaps = 5usize;
    let gap = (avail / ngaps).max(1);
    let extra = if avail >= ngaps { avail % ngaps } else { 0 };
    let gap_at = |i: usize| " ".repeat(gap + if i < extra { 1 } else { 0 });

    // 单格 → 多 span(显存用量含块条;其余单 span 已对齐补齐)
    let cell_spans = |i: usize, text_spans: Vec<Span<'static>>, cur_w: usize| -> Vec<Span<'static>> {
        // text_spans 已是该列内容;补齐到 w[i](按对齐)
        let pad_n = w[i].saturating_sub(cur_w);
        let mut out = Vec::new();
        match aligns[i] {
            Alignment::Right => {
                if pad_n > 0 {
                    out.push(Span::raw(" ".repeat(pad_n)));
                }
                out.extend(text_spans);
            }
            Alignment::Center => {
                let l = pad_n / 2;
                if l > 0 {
                    out.push(Span::raw(" ".repeat(l)));
                }
                out.extend(text_spans);
                if pad_n - l > 0 {
                    out.push(Span::raw(" ".repeat(pad_n - l)));
                }
            }
            _ => {
                out.extend(text_spans);
                if pad_n > 0 {
                    out.push(Span::raw(" ".repeat(pad_n)));
                }
            }
        }
        out
    };
    let assemble = |cells: Vec<Vec<Span<'static>>>| -> Vec<Span<'static>> {
        let mut spans = vec![Span::raw(" ")];
        for (i, c) in cells.into_iter().enumerate() {
            spans.extend(c);
            if i < 5 {
                spans.push(Span::raw(gap_at(i)));
            }
        }
        spans
    };

    let mut lines: Vec<Line> = Vec::new();
    // 表头
    let head_cells: Vec<Vec<Span>> = (0..6)
        .map(|i| {
            cell_spans(i, vec![Span::styled(headers[i].to_string(), hdr_style())], str_w(headers[i]))
        })
        .collect();
    lines.push(Line::from(assemble(head_cells)));
    lines.push(hrule(iw));

    for g in &snap.gpus {
        let mratio = if g.mt > 0 { g.mu * 100 / g.mt } else { 0 };
        // g.hot 已在采样端做过迟滞(≥85 亮,<82 灭),这里直接用,不会闪
        let idx_style = Style::default().add_modifier(Modifier::BOLD);
        // 显存用量:块条 + GB
        let gb = gb_of(g);
        let mut vram: Vec<Span> = Vec::new();
        let mut vw = 0;
        if show_membar {
            vram.extend(cell_bar(mratio as f64, 10));
            vram.push(Span::raw(" "));
            vw += 11;
        }
        vw += str_w(&gb);
        vram.push(Span::styled(gb, pct_style(mratio as f64)));

        let cells: Vec<Vec<Span>> = vec![
            cell_spans(0, vec![Span::styled(g.idx.clone(), idx_style)], str_w(&g.idx)),
            cell_spans(1, vec![Span::styled(g.name.clone(), Style::default())], str_w(&g.name)),
            cell_spans(2, vram, vw),
            cell_spans(3, vec![Span::styled(format!("{}%", g.util), pct_style(g.util as f64))], str_w(&format!("{}%", g.util))),
            {
                let t = format!("{}°C", g.temp);
                let cw = str_w(&t);
                // 高温只高亮温度这一格(整行反色太晃眼)
                let st = if g.hot {
                    Style::default().fg(Color::Red).add_modifier(Modifier::REVERSED | Modifier::BOLD)
                } else {
                    temp_style(g.temp)
                };
                cell_spans(4, vec![Span::styled(t, st)], cw)
            },
            {
                let pw_pct = if g.plim > 0 { (g.pw * 100 / g.plim) as f64 } else { 0.0 };
                let t = format!("{}/{}W", g.pw, g.plim);
                let cw = str_w(&t);
                cell_spans(5, vec![Span::styled(t, pct_style(pw_pct))], cw)
            },
        ];
        lines.push(Line::from(assemble(cells)));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_gpu_procs(f: &mut Frame, area: Rect, snap: &Snapshot, me: &str) {
    // 无竖线(SIMPLE_HEAVY 风格);第一列=序号(无列名,1 字宽的左 gutter);
    // 表头下粗 ━ 横线;卡间用细 ─ 分隔;COMMAND 末列按宽省略。
    let block = panel("GPU 进程", Color::Yellow);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let iw = inner.width as usize;

    // 列:idx(gutter) PID USER GPU% VRAM CPU% TIME COMMAND
    let (w_pid, w_user, w_gpu, w_vram, w_cpu, w_time) = (7, 9, 5, 7, 6, 7); // w_cpu=6:满载可达 11200%
    // 行布局:1空 + idx(1) + 2空(gutter) + 各列(列间 1 空) + COMMAND
    // 表头/行的固定前缀宽度(到 COMMAND 前)
    let fixed = 1 + 1 + 2 + w_pid + 1 + w_user + 1 + w_gpu + 1 + w_vram + 1 + w_cpu + 1 + w_time + 1;
    let cmd_avail = iw.saturating_sub(fixed);

    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for p in &snap.gpu_procs {
        *counts.entry(p.gpu.clone()).or_insert(0) += 1;
    }

    let mut lines: Vec<Line> = Vec::new();
    // 表头(序号列空白)
    lines.push(Line::from(vec![
        Span::raw(" "),
        Span::raw(" "),      // idx 占位(无列名)
        Span::raw("  "),     // gutter
        Span::styled(pad("PID", w_pid, Alignment::Right), hdr_style()),
        Span::raw(" "),
        Span::styled(pad("USER", w_user, Alignment::Left), hdr_style()),
        Span::raw(" "),
        Span::styled(pad("GPU%", w_gpu, Alignment::Right), hdr_style()),
        Span::raw(" "),
        Span::styled(pad("VRAM", w_vram, Alignment::Right), hdr_style()),
        Span::raw(" "),
        Span::styled(pad("CPU%", w_cpu, Alignment::Right), hdr_style()),
        Span::raw(" "),
        Span::styled(pad("TIME", w_time, Alignment::Right), hdr_style()),
        Span::raw(" "),
        Span::styled("COMMAND", hdr_style()),
    ]));
    lines.push(hrule(iw)); // 表头下粗横线

    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut prev: Option<String> = None;
    for p in &snap.gpu_procs {
        if let Some(pv) = &prev {
            if pv != &p.gpu {
                lines.push(Line::from(Span::styled("─".repeat(iw), dim()))); // 卡间细横线
            }
        }
        prev = Some(p.gpu.clone());
        let c = seen.entry(p.gpu.clone()).or_insert(0);
        *c += 1;
        let mid = (counts[&p.gpu] + 1) / 2;
        let show_g = if *c == mid { p.gpu.clone() } else { String::new() };
        let base = me_style(&p.user, me).unwrap_or_default();
        let (sm_txt, sm_st) = if p.sm == "-" || p.sm.is_empty() {
            ("-".to_string(), dim())
        } else {
            (format!("{}%", p.sm), pct_style(pi(&p.sm)))
        };
        let cmd = truncate_w(&crate::parse::shorten_cmd(&p.cmd, home()), cmd_avail);
        let spans: Vec<Span> = vec![
            Span::raw(" "),
            Span::styled(pad(&show_g, 1, Alignment::Center), Style::default().add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(pad(&p.pid, w_pid, Alignment::Right), base),
            Span::raw(" "),
            Span::styled(pad(&p.user, w_user, Alignment::Left), base),
            Span::raw(" "),
            Span::styled(pad(&sm_txt, w_gpu, Alignment::Right), sm_st),
            Span::raw(" "),
            Span::styled(pad(&format!("{}M", p.fb), w_vram, Alignment::Right), Style::default().fg(Color::Cyan)),
            Span::raw(" "),
            Span::styled(pad(&format!("{}%", p.pcpu), w_cpu, Alignment::Right), base),
            Span::raw(" "),
            Span::styled(pad(&p.etime, w_time, Alignment::Right), dim()),
            Span::raw(" "),
            Span::styled(cmd, base),
        ];
        lines.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

/// CPU 进程表(SIMPLE_HEAVY:表头对齐 + 全宽 ━ 横线 + 行,无竖线)。
fn cpu_lines<'a>(procs: &[&CpuProc], me: &str, with_swap: bool, inner_w: usize) -> Vec<Line<'a>> {
    // 列宽
    let (w_pid, w_user, w_cpu, w_mem, w_res, w_swap, w_time) = (7, 10, 6, 5, 6, 6, 7); // w_cpu=6:满载可达 11200%
    let fixed = w_pid + w_user + w_cpu + w_mem + w_res + w_time + if with_swap { w_swap } else { 0 };
    let ncols = if with_swap { 8 } else { 7 };
    let cmd_w = inner_w.saturating_sub(1 + fixed + (ncols - 1));
    let sp = || Span::raw(" ");

    // 表头(列名 + 对齐)
    let mut head_cols: Vec<(&str, usize, Alignment)> = vec![
        ("PID", w_pid, Alignment::Right),
        ("USER", w_user, Alignment::Left),
        ("CPU%", w_cpu, Alignment::Right),
        ("MEM%", w_mem, Alignment::Right),
        ("RES", w_res, Alignment::Right),
    ];
    if with_swap {
        head_cols.push(("SWAP", w_swap, Alignment::Right));
    }
    head_cols.push(("TIME", w_time, Alignment::Right));

    let mut lines: Vec<Line> = Vec::new();
    let mut hs: Vec<Span> = vec![sp()];
    for (name, wd, al) in &head_cols {
        hs.push(Span::styled(pad(name, *wd, *al), hdr_style()));
        hs.push(sp());
    }
    hs.push(Span::styled("COMMAND", hdr_style()));
    lines.push(Line::from(hs));
    lines.push(hrule(inner_w));

    for p in procs {
        let base = me_style(&p.user, me).unwrap_or_default();
        let mut spans: Vec<Span> = vec![sp()];
        spans.push(Span::styled(pad(&p.pid, w_pid, Alignment::Right), base));
        spans.push(sp());
        spans.push(Span::styled(pad(&p.user, w_user, Alignment::Left), base));
        spans.push(sp());
        spans.push(Span::styled(pad(&format!("{}%", p.pcpu), w_cpu, Alignment::Right), pct_style(pi(&p.pcpu))));
        spans.push(sp());
        spans.push(Span::styled(pad(&format!("{}%", p.pmem), w_mem, Alignment::Right), base));
        spans.push(sp());
        spans.push(Span::styled(pad(&crate::parse::fmt_rss(p.rss), w_res, Alignment::Right), base));
        spans.push(sp());
        if with_swap {
            let (txt, st) = if p.swap == 0 {
                ("-".to_string(), dim())
            } else {
                (crate::parse::fmt_rss(p.swap), Style::default().fg(Color::Yellow))
            };
            spans.push(Span::styled(pad(&txt, w_swap, Alignment::Right), st));
            spans.push(sp());
        }
        spans.push(Span::styled(pad(&p.etime, w_time, Alignment::Right), dim()));
        spans.push(sp());
        spans.push(Span::styled(truncate_w(&crate::parse::shorten_cmd(&p.cmd, home()), cmd_w), base));
        lines.push(Line::from(spans));
    }
    lines
}

fn home() -> &'static str {
    use std::sync::OnceLock;
    static HOME: OnceLock<String> = OnceLock::new();
    HOME.get_or_init(|| std::env::var("HOME").unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::{CpuProc, GpuProc, Snapshot};
    use crate::parse::Gpu;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn mk_snap(ngpu: usize, ngproc: usize, ncproc: usize, sw_total: f64) -> Snapshot {
        let gpus = (0..ngpu)
            .map(|i| Gpu {
                idx: i.to_string(),
                name: "A800 80GB".into(),
                temp: 60 + (i as i64) * 10,
                util: 50,
                mu: 40000,
                mt: 81920,
                pw: 200,
                plim: 300,
                hot: i == 2,
            })
            .collect();
        let gpu_procs = (0..ngproc)
            .map(|i| GpuProc {
                gpu: (i % ngpu.max(1)).to_string(),
                pid: (1000 + i).to_string(),
                user: "u".into(),
                sm: if i % 3 == 0 { "-".into() } else { "50".into() },
                fb: "1000".into(),
                pcpu: "100".into(),
                etime: "1h2m".into(),
                cmd: "/opt/conda/bin/python train.py --flag /home/x/中文路径/data".into(),
            })
            .collect();
        let cpu_procs = (0..ncproc)
            .map(|i| CpuProc {
                pid: (2000 + i).to_string(),
                user: "u".into(),
                pcpu: "100".into(),
                pmem: "1.0".into(),
                rss: 100000,
                swap: (i as i64) * 50000,
                etime: "5m".into(),
                cmd: "python y.py".into(),
            })
            .collect();
        Snapshot {
            gpus,
            gpu_procs,
            cpu_procs,
            cpu: 40.0,
            mem: (98.0, 256.0),
            swap: (sw_total * 0.4, sw_total, sw_total > 0.0),
            net: (1.0, 2.0),
            uptime: 50.0 * 86400.0 + 4.0 * 3600.0,
            load: (13.48, 18.18, 26.75),
            driver: "570.211.01".into(),
            cuda: "12.8".into(),
        }
    }

    fn draw(snap: &Snapshot, w: u16, h: u16, mode: Mode, rev: bool) { draw_f(snap, w, h, mode, rev, None, None) }

    fn draw_f(snap: &Snapshot, w: u16, h: u16, mode: Mode, rev: bool, filter: Option<&Filter>, typing: Option<&str>) {
        let backend = TestBackend::new(w, h);
        let mut term = Terminal::new(backend).unwrap();
        let paused = rev; // 顺带覆盖 paused 两种取值
        term.draw(|f| render(f, snap, mode, rev, "u", "host", "2026-06-30 00:00:00", paused, filter, typing))
            .unwrap();
    }

    #[test]
    fn smoke_variants() {
        let snaps = [mk_snap(2, 3, 5, 61.0), mk_snap(0, 0, 0, 0.0), mk_snap(4, 12, 60, 61.0), mk_snap(8, 0, 1, 0.0)];
        for snap in &snaps {
            for &(w, h) in &[(50u16, 18u16), (80, 24), (120, 30), (160, 40), (220, 50)] {
                for mode in [Mode::Full, Mode::Cpu] {
                    for rev in [false, true] {
                        draw(snap, w, h, mode, rev);
                    }
                }
            }
        }
    }

    #[test]
    fn t_filter_hit() {
        let u = Filter::User("user01".into());
        assert!(u.hit("user01", "python train.py"));
        assert!(!u.hit("user010", "x"));          // 精确匹配,不是前缀
        assert!(!u.hit("user02", "/home/user01/a.py")); // 别人跑我路径下的脚本,不算我的
        let t = Filter::Text("PyThOn".into());
        assert!(t.hit("bob", "/usr/bin/python3 a.py")); // 命令命中,不分大小写
        assert!(t.hit("pythonuser", "sleep"));          // 用户名命中
        assert!(!t.hit("bob", "sleep 1"));
    }

    #[test]
    fn t_load_style() {
        let (g, y, r) = (
            Style::default().fg(Color::Green),
            Style::default().fg(Color::Yellow),
            Style::default().fg(Color::Red),
        );
        // 阈值:<0.7×核 绿,0.7~1.0× 黄,≥1.0× 红(以 100 核为参照)
        assert_eq!(load_style(0.0, 100), g);
        assert_eq!(load_style(69.0, 100), g);
        assert_eq!(load_style(70.0, 100), y);
        assert_eq!(load_style(99.9, 100), y);
        assert_eq!(load_style(100.0, 100), r);
        assert_eq!(load_style(250.0, 100), r);
    }

    #[test]
    fn smoke_filter_and_typing() {
        let snap = mk_snap(4, 6, 30, 61.0);
        let filters = [
            None,
            Some(Filter::User("u".into())),      // 命中全部(mk_snap 里 user 都是 "u")
            Some(Filter::User("nobody".into())), // 命中 0 条:面板不能消失
            Some(Filter::Text("python".into())),
        ];
        for fl in &filters {
            for typ in [None, Some(""), Some("py")] {
                for &(w, h) in &[(60u16, 20u16), (120, 30), (200, 45)] {
                    for mode in [Mode::Full, Mode::Cpu] {
                        draw_f(&snap, w, h, mode, false, fl.as_ref(), typ);
                    }
                }
            }
        }
    }

    #[test]
    fn smoke_edge() {
        let mut snap = mk_snap(1, 1, 1, 0.0);
        snap.mem = (0.0, 0.0);
        draw(&snap, 120, 30, Mode::Full, false);
        draw(&snap, 120, 30, Mode::Cpu, false);
    }
}
