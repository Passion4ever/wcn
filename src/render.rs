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

pub fn render(f: &mut Frame, snap: &Snapshot, mode: Mode, rev: bool, me: &str, host: &str, now: &str) {
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
        render_header(f, chunks[0], host, now);
        let n = h.saturating_sub(6).max(3);
        let mut procs: Vec<&CpuProc> = snap.cpu_procs.iter().collect();
        if rev {
            procs.reverse();
        }
        let shown = procs.len().min(n);
        let order = if rev { "升序" } else { "降序" };
        let title = format!("CPU 进程 · 共 {},显示 {}({})", snap.cpu_procs.len(), shown, order);
        let inner_w = chunks[1].width.saturating_sub(2) as usize;
        let lines = cpu_lines(&procs[..shown], me, true, inner_w);
        f.render_widget(Paragraph::new(lines).block(panel(&title, Color::Blue)), chunks[1]);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled("  [c] 返回   [r] 反序   [q] 退出", dim()))),
            chunks[2],
        );
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

    let remaining = h.saturating_sub(2 + top_h);
    let cpu_chrome = 4; // 边框2 + 表头1 + 横线1
    let want_cpu = snap.cpu_procs.len().min(10);
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

    render_header(f, chunks[0], host, now);
    render_top(f, chunks[1], snap, side_by_side);
    render_gpu_procs(f, chunks[2], snap, me);
    if cpu_panel_h > 0 {
        let procs: Vec<&CpuProc> = snap.cpu_procs.iter().take(cpu_rows).collect();
        let inner_w = chunks[3].width.saturating_sub(2) as usize;
        let lines = cpu_lines(&procs, me, false, inner_w);
        f.render_widget(Paragraph::new(lines).block(panel("CPU 进程 Top 10", Color::Blue)), chunks[3]);
    }
    let hint = chunks.last().unwrap();
    f.render_widget(
        Paragraph::new(Line::from(Span::styled("  [c] 只看CPU进程   [q] 退出", dim()))),
        *hint,
    );
}

fn render_header(f: &mut Frame, area: Rect, host: &str, now: &str) {
    let left = Line::from(vec![
        Span::styled("wcn2", hdr_style()),
        Span::styled(format!(" @ {}", host), dim()),
    ]);
    let right = Line::from(Span::styled(format!("{}  ·  刷新 0.8s", now), dim())).alignment(Alignment::Right);
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
    let block = panel("GPU", Color::Green);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let iw = inner.width as usize;
    if snap.gpus.is_empty() {
        f.render_widget(Paragraph::new(" 无 NVIDIA GPU / nvidia-smi 不可用"), inner);
        return;
    }
    let show_membar = iw >= 80;
    // 列宽:GPU3 名称10 显存用量(fill) 利用率6 温度6 功耗9;5 个列间隔 + 前导1
    let wmem = iw.saturating_sub(40).max(8);
    let sp = || Span::raw(" ");

    let mut lines: Vec<Line> = Vec::new();
    // 表头(对齐与数据一致)
    lines.push(Line::from(vec![
        sp(),
        Span::styled(pad("GPU", 3, Alignment::Center), hdr_style()),
        sp(),
        Span::styled(pad("名称", 10, Alignment::Left), hdr_style()),
        sp(),
        Span::styled(pad("显存用量", wmem, Alignment::Left), hdr_style()),
        sp(),
        Span::styled(pad("利用率", 6, Alignment::Right), hdr_style()),
        sp(),
        Span::styled(pad("温度", 6, Alignment::Right), hdr_style()),
        sp(),
        Span::styled(pad("功耗", 9, Alignment::Right), hdr_style()),
    ]));
    lines.push(hrule(iw));

    for g in &snap.gpus {
        let mratio = if g.mt > 0 { g.mu * 100 / g.mt } else { 0 };
        let crit = g.temp >= 85;
        let idx_style = if crit {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        } else {
            Style::default().add_modifier(Modifier::BOLD)
        };
        // 显存用量单元格:块条 + GB,补到 wmem 宽
        let mut vram: Vec<Span> = Vec::new();
        let mut vw = 0usize;
        if show_membar {
            vram.extend(cell_bar(mratio as f64, 10));
            vram.push(Span::raw(" "));
            vw += 11;
        }
        let gb = format!("{}/{}G", g.mu / 1024, g.mt / 1024);
        vw += str_w(&gb);
        vram.push(Span::styled(gb, pct_style(mratio as f64)));
        if wmem > vw {
            vram.push(Span::raw(" ".repeat(wmem - vw)));
        }
        let pw_pct = if g.plim > 0 { (g.pw * 100 / g.plim) as f64 } else { 0.0 };
        let mut spans: Vec<Span> = vec![
            sp(),
            Span::styled(pad(&g.idx, 3, Alignment::Center), idx_style),
            sp(),
            Span::styled(pad(&g.name, 10, Alignment::Left), Style::default()),
            sp(),
        ];
        spans.extend(vram);
        spans.extend([
            sp(),
            Span::styled(pad(&format!("{}%", g.util), 6, Alignment::Right), pct_style(g.util as f64)),
            sp(),
            Span::styled(pad(&format!("{}°C", g.temp), 6, Alignment::Right), temp_style(g.temp)),
            sp(),
            Span::styled(pad(&format!("{}/{}W", g.pw, g.plim), 9, Alignment::Right), pct_style(pw_pct)),
        ]);
        if crit {
            for s in spans.iter_mut() {
                s.style = s.style.add_modifier(Modifier::REVERSED);
            }
        }
        lines.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

// GPU 进程表固定列宽
const GP_W: [usize; 7] = [3, 7, 9, 5, 7, 5, 7];

fn grid_rule(inner: usize) -> Line<'static> {
    let mut s = String::new();
    s.push_str(&"─".repeat(1 + GP_W[0] + 1));
    for w in &GP_W[1..] {
        s.push('┼');
        s.push_str(&"─".repeat(1 + w + 1));
    }
    s.push('┼');
    let used = s.chars().count();
    if inner > used {
        s.push_str(&"─".repeat(inner - used));
    }
    Line::from(Span::styled(s.chars().take(inner).collect::<String>(), dim()))
}

fn render_gpu_procs(f: &mut Frame, area: Rect, snap: &Snapshot, me: &str) {
    let block = panel("GPU 进程", Color::Yellow);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let iw = inner.width as usize;
    let sep = || Span::styled(" │ ", dim());

    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for p in &snap.gpu_procs {
        *counts.entry(p.gpu.clone()).or_insert(0) += 1;
    }
    let headers = ["GPU", "PID", "USER", "GPU%", "VRAM", "CPU%", "TIME"];
    let aligns = [
        Alignment::Center,
        Alignment::Right,
        Alignment::Left,
        Alignment::Right,
        Alignment::Right,
        Alignment::Right,
        Alignment::Right,
    ];
    let mut lines: Vec<Line> = Vec::new();
    let mut hs: Vec<Span> = vec![Span::raw(" ")];
    for i in 0..7 {
        hs.push(Span::styled(pad(headers[i], GP_W[i], aligns[i]), hdr_style()));
        hs.push(sep());
    }
    hs.push(Span::styled("COMMAND", hdr_style()));
    lines.push(Line::from(hs));
    lines.push(grid_rule(iw));

    let cmd_avail = iw.saturating_sub(1 + GP_W.iter().sum::<usize>() + GP_W.len() * 3);
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut prev: Option<String> = None;
    for p in &snap.gpu_procs {
        if let Some(pv) = &prev {
            if pv != &p.gpu {
                lines.push(grid_rule(iw));
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
        let cells: [(String, Style); 7] = [
            (pad(&show_g, GP_W[0], Alignment::Center), Style::default().add_modifier(Modifier::BOLD)),
            (pad(&p.pid, GP_W[1], Alignment::Right), base),
            (pad(&p.user, GP_W[2], Alignment::Left), base),
            (pad(&sm_txt, GP_W[3], Alignment::Right), sm_st),
            (pad(&format!("{}M", p.fb), GP_W[4], Alignment::Right), Style::default().fg(Color::Cyan)),
            (pad(&format!("{}%", p.pcpu), GP_W[5], Alignment::Right), base),
            (pad(&p.etime, GP_W[6], Alignment::Right), dim()),
        ];
        let mut spans: Vec<Span> = vec![Span::raw(" ")];
        for (txt, st) in cells {
            spans.push(Span::styled(txt, st));
            spans.push(sep());
        }
        spans.push(Span::styled(cmd, base));
        lines.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

/// CPU 进程表(SIMPLE_HEAVY:表头对齐 + 全宽 ━ 横线 + 行,无竖线)。
fn cpu_lines<'a>(procs: &[&CpuProc], me: &str, with_swap: bool, inner_w: usize) -> Vec<Line<'a>> {
    // 列宽
    let (w_pid, w_user, w_cpu, w_mem, w_res, w_swap, w_time) = (7, 10, 5, 5, 6, 6, 7);
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
        }
    }

    fn draw(snap: &Snapshot, w: u16, h: u16, mode: Mode, rev: bool) {
        let backend = TestBackend::new(w, h);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, snap, mode, rev, "u", "host", "2026-06-30 00:00:00"))
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
    fn smoke_edge() {
        let mut snap = mk_snap(1, 1, 1, 0.0);
        snap.mem = (0.0, 0.0);
        draw(&snap, 120, 30, Mode::Full, false);
        draw(&snap, 120, 30, Mode::Cpu, false);
    }
}
