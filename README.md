# wcn2-rs — wcn2 的 Rust 重写

终端 GPU + 系统监控的 Rust 版,产出**单个静态二进制**,`scp` 到任何 x86_64 Linux 即可运行,
无需 Python/conda/任何环境。行为对齐 Python 版(`/home/user01/wcn2/wcn2.py`)。

## 构建(本机走 socks5h 代理)

```bash
# 一次性:工具链 + musl target(走 socks5h)
curl --proxy socks5h://USER:PASS@127.0.0.1:2080 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
# ~/.cargo/config.toml 里设 [http] proxy = "socks5h://USER:PASS@127.0.0.1:2080"
rustup target add x86_64-unknown-linux-musl

# 构建静态二进制
cargo build --release --target x86_64-unknown-linux-musl
# 产物:target/x86_64-unknown-linux-musl/release/wcn2 (~1.3M, statically linked)
```

`ldd` 应显示 `statically linked` —— 可直接 `scp` 到任何 Linux 运行。

## 依赖

- ratatui + crossterm(纯 Rust;musl 全静态无需 C 编译器)。
- 运行时:`nvidia-smi`(GPU 数据,v1 走子进程;v2 计划改 NVML)、`ps`、`date`、`/proc`。

## 按键 / 界面

与 Python 版一致:`c` 切 CPU 单页(多 SWAP 列)、`r` 反序(仅 CPU 单页)、`q`/`Ctrl-C` 退出。
面板:状态栏、系统(CPU/内存/交换/网速)、GPU 概览、GPU 进程(按卡分组)、CPU 进程。
自适应宽高 + 最小尺寸兜底;配色相对主题(深浅自适应)。

## 模块

- `parse.rs` — 纯解析/格式化(无 IO),含单测。
- `collect.rs` — 采样,组装 `Snapshot`(瞬时 CPU%、per-proc swap)。
- `render.rs` — ratatui 渲染,含 TestBackend 冒烟。
- `main.rs` — 事件循环 / 后台采样线程 / 终端守卫。

## 测试

```bash
cargo test    # 15 项:纯函数单测 + 渲染冒烟
```

## 状态

v1 功能完整、与 Python 版行为对齐(细节视觉仍在对照打磨)。NVML 提速为 v2。
