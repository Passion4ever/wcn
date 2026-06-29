// ratatui 渲染。行为对照 Python wcn2.py 的 render()。
// 说明:ratatui 表格需显式列宽、无内置网格线;GPU 进程表的卡间分隔用分隔行实现,
// 系统面板的填充条用 LineGauge。配色全部相对主题(不设 fg 即用终端默认;dim/reversed/语义色)。
use crate::collect::{CpuProc, Snapshot};
use ratatui::layout::{Alignment, Constraint, Direction, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Paragraph, Row, Table};
use ratatui::Frame;

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Full,
    Cpu,
}

fn dim() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

/// 利用率配色:0/空=dim,<50 绿,<85 黄,否则红。
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

/// 固定宽度 unicode 小条(用于表格单元格,如 GPU 显存用量)。
fn cell_bar(pct: f64, width: usize) -> Vec<Span<'static>> {
    let pct = pct.clamp(0.0, 100.0);
    let fill = ((pct / 100.0 * width as f64).round() as usize).min(width);
    vec![
        Span::styled("█".repeat(fill), pct_style(pct)),
        Span::styled("░".repeat(width - fill), dim()),
    ]
}

/// 字符显示宽度(粗略东亚宽字符=2),给命令按显示宽截断用。
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

/// 按显示宽截断,超长结尾加 …(留 1 列给省略号)。
fn truncate_w(s: &str, max: usize) -> String {
    let total: usize = s.chars().map(disp_w).sum();
    if total <= max {
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

fn r_cell(s: String, style: Style) -> Cell<'static> {
    Cell::from(Line::from(Span::styled(s, style)).alignment(Alignment::Right))
}
fn l_cell(s: String, style: Style) -> Cell<'static> {
    Cell::from(Line::from(Span::styled(s, style)))
}

const HEADER_STYLE: fn() -> Style = || Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);

pub fn render(f: &mut Frame, snap: &Snapshot, mode: Mode, rev: bool, me: &str, host: &str, now: &str) {
    let area = f.area();
    let w = area.width as usize;
    let h = area.height as usize;
    let ngpu = snap.gpus.len();

    // ===== 最小尺寸兜底 =====
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
            x: area.x + (area.width.saturating_sub(bw)) / 2,
            y: area.y + (area.height.saturating_sub(bh)) / 2,
            width: bw,
            height: bh,
        };
        let p = Paragraph::new(msg)
            .alignment(Alignment::Center)
            .block(panel("", Color::Red));
        f.render_widget(p, rect);
        return;
    }

    // ===== CPU 单页 =====
    if mode == Mode::Cpu {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)])
            .split(area);
        render_header(f, chunks[0], host, now);
        let n = (h.saturating_sub(6)).max(3);
        let mut procs: Vec<&CpuProc> = snap.cpu_procs.iter().collect();
        if rev {
            procs.reverse();
        }
        let order = if rev { "升序" } else { "降序" };
        let shown = procs.len().min(n);
        let title = format!("CPU 进程 · 共 {},显示 {}({})", snap.cpu_procs.len(), shown, order);
        let table = cpu_table(&procs[..shown], me, true, w.saturating_sub(2)).block(panel(&title, Color::Blue));
        f.render_widget(table, chunks[1]);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled("  [c] 返回   [r] 反序   [q] 退出", dim()))),
            chunks[2],
        );
        return;
    }

    // ===== 完整视图 =====
    // 高度预算:header 1 + 顶部区 top_h + gpu进程 + cpu进程 + hint 1
    let sys_rows = 2 + if snap.swap.1 > 0.0 { 1 } else { 0 }; // CPU/内存(/交换)/网络
    let sys_h = sys_rows + 1 + 2; // 3或4行 + 边框2... 实际行数=sys_rows+1(网络始终在)
    let sys_rows_total = 3 + if snap.swap.1 > 0.0 { 1 } else { 0 };
    let sys_panel_h = sys_rows_total + 2;
    let gpu_overview_h = ngpu + 1 + 2; // header + 各卡 + 边框
    let side_by_side = w >= 120;
    let top_h = if side_by_side {
        sys_panel_h.max(gpu_overview_h)
    } else {
        sys_panel_h + gpu_overview_h
    };
    let _ = sys_h;

    // GPU 进程面板高度:边框2 + 表头1 + 进程行 + 卡间分隔行
    let ncards = {
        let mut s = std::collections::BTreeSet::new();
        for p in &snap.gpu_procs {
            s.insert(p.gpu.clone());
        }
        s.len()
    };
    let gpu_procs_h = 2 + 1 + snap.gpu_procs.len() + ncards.saturating_sub(1);

    let remaining = h.saturating_sub(2 + top_h); // 减 header + hint + top
    let cpu_chrome = 3; // 边框2 + 表头1
    let want_cpu = snap.cpu_procs.len().min(10);
    let cpu_rows = (remaining.saturating_sub(gpu_procs_h + cpu_chrome)).min(want_cpu);
    let cpu_panel_h = if cpu_rows >= 1 { cpu_rows + cpu_chrome } else { 0 };

    let mut constraints = vec![
        Constraint::Length(1),               // header
        Constraint::Length(top_h as u16),    // 顶部区
        Constraint::Length(gpu_procs_h as u16), // GPU 进程
    ];
    if cpu_panel_h > 0 {
        constraints.push(Constraint::Length(cpu_panel_h as u16));
    }
    constraints.push(Constraint::Min(0)); // filler 把提示行顶到底
    constraints.push(Constraint::Length(1)); // hint
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    render_header(f, chunks[0], host, now);
    render_top(f, chunks[1], snap, side_by_side, w);
    render_gpu_procs(f, chunks[2], snap, me);
    let mut idx = 3;
    if cpu_panel_h > 0 {
        let procs: Vec<&CpuProc> = snap.cpu_procs.iter().take(cpu_rows).collect();
        let table = cpu_table(&procs, me, false, w.saturating_sub(2)).block(panel("CPU 进程 Top 10", Color::Blue));
        f.render_widget(table, chunks[idx]);
        idx += 1;
    }
    let _ = idx;
    let hint = chunks.last().unwrap();
    f.render_widget(
        Paragraph::new(Line::from(Span::styled("  [c] 只看CPU进程   [q] 退出", dim()))),
        *hint,
    );
}

fn render_header(f: &mut Frame, area: Rect, host: &str, now: &str) {
    let left = Line::from(vec![
        Span::styled("wcn2", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" @ {}", host), dim()),
    ]);
    let right = Line::from(Span::styled(format!("{}  ·  刷新 0.8s", now), dim()))
        .alignment(Alignment::Right);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);
    f.render_widget(Paragraph::new(left), cols[0]);
    f.render_widget(Paragraph::new(right), cols[1]);
}

fn render_top(f: &mut Frame, area: Rect, snap: &Snapshot, side_by_side: bool, _w: usize) {
    if side_by_side {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(34), Constraint::Min(0)])
            .split(area);
        render_sys(f, cols[0], snap);
        render_gpu_overview(f, cols[1], snap);
    } else {
        let sys_rows_total = 3 + if snap.swap.1 > 0.0 { 1 } else { 0 };
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length((sys_rows_total + 2) as u16),
                Constraint::Min(0),
            ])
            .split(area);
        render_sys(f, rows[0], snap);
        render_gpu_overview(f, rows[1], snap);
    }
}

fn render_sys(f: &mut Frame, area: Rect, snap: &Snapshot) {
    let block = panel("系统", Color::Cyan);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let (mu, mt) = snap.mem;
    let mem_pct = if mt > 0.0 { mu / mt * 100.0 } else { 0.0 };
    let cpu = snap.cpu;
    let (sw_used, sw_total, sw_active) = snap.swap;
    let (down, up) = snap.net;

    let mut rows: Vec<(Line, Option<f64>, Line)> = Vec::new();
    rows.push((
        Line::from(Span::styled("CPU", Style::default().fg(Color::Cyan))),
        Some(cpu),
        Line::from(Span::styled(format!("{:.0}%", cpu), pct_style(cpu))).alignment(Alignment::Right),
    ));
    rows.push((
        Line::from(Span::styled("内存", Style::default().fg(Color::Magenta))),
        Some(mem_pct),
        Line::from(Span::styled(format!("{:.0}/{:.0}G", mu, mt), pct_style(mem_pct)))
            .alignment(Alignment::Right),
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
            Line::from(Span::styled("交换", Style::default().fg(col))),
            Some(sw_pct),
            Line::from(Span::styled(format!("{:.0}/{:.0}G{}", sw_used, sw_total, tail), Style::default().fg(col)))
                .alignment(Alignment::Right),
        ));
    }
    rows.push((
        Line::from(Span::styled("网络", Style::default().fg(Color::Blue))),
        None,
        Line::from(vec![
            Span::styled(format!("↓{:.1} ", down), Style::default().fg(Color::Green)),
            Span::styled(format!("↑{:.1} ", up), Style::default().fg(Color::Yellow)),
            Span::styled("MB/s", dim()),
        ])
        .alignment(Alignment::Right),
    ));

    let row_areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Length(1); rows.len()])
        .flex(Flex::Center) // 并排时与 GPU 表等高:行组垂直居中
        .split(inner);
    for (i, (label, ratio, value)) in rows.into_iter().enumerate() {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(5), Constraint::Min(4), Constraint::Length(10)])
            .split(row_areas[i]);
        f.render_widget(Paragraph::new(label), cols[0]);
        if let Some(r) = ratio {
            // 粗块条(█/░)填满中列,与 Python 一致
            let bar = cell_bar(r, cols[1].width as usize);
            f.render_widget(Paragraph::new(Line::from(bar)), cols[1]);
        }
        f.render_widget(Paragraph::new(value), cols[2]);
    }
}

fn render_gpu_overview(f: &mut Frame, area: Rect, snap: &Snapshot) {
    if snap.gpus.is_empty() {
        let p = Paragraph::new("无 NVIDIA GPU / nvidia-smi 不可用").block(panel("GPU", Color::Green));
        f.render_widget(p, area);
        return;
    }
    let show_membar = area.width as usize >= 80;
    let mut rows = Vec::new();
    for g in &snap.gpus {
        let mratio = if g.mt > 0 { g.mu * 100 / g.mt } else { 0 };
        let crit = g.temp >= 85;
        let idx_style = if crit {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        } else {
            Style::default().add_modifier(Modifier::BOLD)
        };
        let mut vram: Vec<Span> = Vec::new();
        if show_membar {
            vram.extend(cell_bar(mratio as f64, 10));
            vram.push(Span::raw(" "));
        }
        vram.push(Span::styled(format!("{}/{}G", g.mu / 1024, g.mt / 1024), pct_style(mratio as f64)));
        let pw_pct = if g.plim > 0 { (g.pw * 100 / g.plim) as f64 } else { 0.0 };
        let mut row = Row::new(vec![
            Cell::from(Line::from(Span::styled(g.idx.clone(), idx_style)).alignment(Alignment::Center)),
            l_cell(g.name.clone(), Style::default()),
            Cell::from(Line::from(vram)),
            r_cell(format!("{}%", g.util), pct_style(g.util as f64)),
            r_cell(format!("{}°C", g.temp), temp_style(g.temp)),
            r_cell(format!("{}/{}W", g.pw, g.plim), pct_style(pw_pct)),
        ]);
        if crit {
            row = row.style(Style::default().add_modifier(Modifier::REVERSED));
        }
        rows.push(row);
    }
    let widths = [
        Constraint::Length(4),
        Constraint::Length(12),
        Constraint::Fill(1),
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Length(10),
    ];
    let table = Table::new(rows, widths)
        .header(Row::new(vec!["GPU", "名称", "显存用量", "利用率", "温度", "功耗"]).style(HEADER_STYLE()))
        .column_spacing(1)
        .block(panel("GPU", Color::Green));
    f.render_widget(table, area);
}

// 固定列宽(字符)。最后一列 COMMAND 占剩余宽度。
const GP_W: [usize; 7] = [3, 7, 9, 5, 7, 5, 7]; // GPU PID USER GPU% VRAM CPU% TIME

/// 按对齐把内容补齐到固定宽(ASCII 列;按 char 数)。超长截断。
fn pad(s: &str, w: usize, align: Alignment) -> String {
    let n = s.chars().count();
    if n >= w {
        return s.chars().take(w).collect();
    }
    let sp = w - n;
    match align {
        Alignment::Right => format!("{}{}", " ".repeat(sp), s),
        Alignment::Center => {
            let l = sp / 2;
            format!("{}{}{}", " ".repeat(l), s, " ".repeat(sp - l))
        }
        _ => format!("{}{}", s, " ".repeat(sp)),
    }
}

/// 横线规则行(表头下 / 卡间分隔):── 与列竖线位置对齐的 ┼ 交叉,末尾铺满至 inner 宽。
fn grid_rule(inner: usize) -> Line<'static> {
    let mut s = String::new();
    s.push_str(&"─".repeat(1 + GP_W[0] + 1)); // 前导空格 + 列 + 竖线前空格
    for w in &GP_W[1..] {
        s.push('┼');
        s.push_str(&"─".repeat(1 + w + 1));
    }
    s.push('┼'); // COMMAND 前的交叉
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

    // 每卡进程数(卡号垂直居中)
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

    // 表头行
    let mut hs: Vec<Span> = vec![Span::raw(" ")];
    for i in 0..7 {
        hs.push(Span::styled(pad(headers[i], GP_W[i], aligns[i]), HEADER_STYLE()));
        hs.push(sep());
    }
    hs.push(Span::styled("COMMAND", HEADER_STYLE()));
    lines.push(Line::from(hs));
    lines.push(grid_rule(iw)); // 表头下横线

    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut prev: Option<String> = None;
    for p in &snap.gpu_procs {
        if let Some(pv) = &prev {
            if pv != &p.gpu {
                lines.push(grid_rule(iw)); // 卡间横线
            }
        }
        prev = Some(p.gpu.clone());
        let c = seen.entry(p.gpu.clone()).or_insert(0);
        *c += 1;
        let mid = (counts[&p.gpu] + 1) / 2;
        let show_g = if *c == mid { p.gpu.clone() } else { String::new() };
        let base = if p.user == me {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let sm_style = if p.sm == "-" || p.sm.is_empty() {
            dim()
        } else {
            pct_style(pi(&p.sm))
        };
        let sm_txt = if p.sm == "-" || p.sm.is_empty() {
            "-".to_string()
        } else {
            format!("{}%", p.sm)
        };
        // COMMAND 可用宽 = inner - 前导1 - 各固定列 - 各 " │ "(7×3)
        let cmd_avail = iw.saturating_sub(1 + GP_W.iter().sum::<usize>() + GP_W.len() * 3);
        let cmd = truncate_w(&crate::parse::shorten_cmd(&p.cmd, home()), cmd_avail);
        let cells: [(String, Style); 7] = [
            (pad(&show_g, GP_W[0], Alignment::Center), Style::default().add_modifier(Modifier::BOLD)),
            (pad(&p.pid, GP_W[1], Alignment::Right), base),
            (pad(&p.user, GP_W[2], Alignment::Left), base),
            (pad(&sm_txt, GP_W[3], Alignment::Right), sm_style),
            (pad(&format!("{}M", p.fb), GP_W[4], Alignment::Right), Style::default().fg(Color::Cyan)),
            (pad(&format!("{}%", p.pcpu), GP_W[5], Alignment::Right), base),
            (pad(&p.etime, GP_W[6], Alignment::Right), dim()),
        ];
        let mut spans: Vec<Span> = vec![Span::raw(" ")];
        for (txt, st) in cells {
            spans.push(Span::styled(txt, st));
            spans.push(sep());
        }
        spans.push(Span::styled(cmd, base)); // 末列,超出由 Paragraph 截断
        lines.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn cpu_table<'a>(procs: &[&CpuProc], me: &str, with_swap: bool, inner_w: usize) -> Table<'a> {
    // COMMAND 可用宽 = 面板内宽 - 各固定列 - 列间距
    let fixed = 8 + 10 + 6 + 6 + 7 + 7 + if with_swap { 8 } else { 0 };
    let ncols = if with_swap { 8 } else { 7 };
    let cmd_w = inner_w.saturating_sub(fixed + (ncols - 1));
    let mut rows: Vec<Row> = Vec::new();
    for p in procs {
        let me_style = if p.user == me {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let mut cells = vec![
            r_cell(p.pid.clone(), Style::default()),
            l_cell(p.user.clone(), Style::default()),
            r_cell(format!("{}%", p.pcpu), pct_style(pi(&p.pcpu))),
            r_cell(format!("{}%", p.pmem), Style::default()),
            r_cell(crate::parse::fmt_rss(p.rss), Style::default()),
        ];
        if with_swap {
            if p.swap == 0 {
                cells.push(r_cell("-".into(), dim()));
            } else {
                cells.push(r_cell(crate::parse::fmt_rss(p.swap), Style::default().fg(Color::Yellow)));
            }
        }
        cells.push(r_cell(p.etime.clone(), dim()));
        cells.push(l_cell(truncate_w(&crate::parse::shorten_cmd(&p.cmd, home()), cmd_w), Style::default()));
        rows.push(Row::new(cells).style(me_style));
    }
    let mut header = vec!["PID", "USER", "CPU%", "MEM%", "RES"];
    let mut widths = vec![
        Constraint::Length(8),
        Constraint::Length(10),
        Constraint::Length(6),
        Constraint::Length(6),
        Constraint::Length(7),
    ];
    if with_swap {
        header.push("SWAP");
        widths.push(Constraint::Length(8));
    }
    header.push("TIME");
    widths.push(Constraint::Length(7));
    header.push("COMMAND");
    widths.push(Constraint::Fill(1));
    Table::new(rows, widths)
        .header(Row::new(header).style(HEADER_STYLE()))
        .column_spacing(1)
}

fn home() -> &'static str {
    // 缓存一次 $HOME
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
                cmd: "/opt/conda/bin/python train.py --flag".into(),
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
        let snaps = [
            mk_snap(2, 3, 5, 61.0),
            mk_snap(0, 0, 0, 0.0),
            mk_snap(4, 12, 60, 61.0),
            mk_snap(8, 0, 1, 0.0),
        ];
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
        // mem 总量 0(除零)、缺数据
        let mut snap = mk_snap(1, 1, 1, 0.0);
        snap.mem = (0.0, 0.0);
        draw(&snap, 120, 30, Mode::Full, false);
        draw(&snap, 120, 30, Mode::Cpu, false);
    }
}
