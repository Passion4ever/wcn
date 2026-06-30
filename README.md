# wcn

终端里的 **GPU + 系统监控**,`top` 与 `nvidia-smi` 的合体。一屏看清:GPU 状态、CPU/内存/网速、谁在占资源。

- **单个静态二进制**:`scp` 到任何 x86_64 Linux 直接跑,不依赖 Python / CUDA toolkit / 任何环境。
- **不闪刷新**、自适应终端宽高、自动适配深/浅色主题。
- Rust + [ratatui](https://github.com/ratatui/ratatui) 实现,~1.3MB,启动快、CPU 占用低。

> 个人自用项目。GPU 数据来自 `nvidia-smi`;CPU/内存/网速读 `/proc`。

## 界面

![wcn 截图](assets/screenshot.png)

- **系统**:CPU 利用率、内存 用/总、(有 swap 时)交换、网速 ↓↑;进度条按阈值绿/黄/红。
  交换行:灰=空、黄=占着、红+`⇅`=**正在换页**(性能在被拖累)。
- **GPU 概览**:每卡 显存用量(条+GB)、利用率、温度、功耗;标题带驱动/CUDA 版本;**温度 ≥85°C 整行高亮**。
- **GPU 进程**:只列占着 GPU 的进程,按卡分组;`VRAM`=进程占的显存。
- **CPU 进程**:按**瞬时** CPU% 降序(读 `/proc/<pid>/stat` 增量,非 ps 生命周期均值);
  `RES`=系统内存,`TIME`=运行时长。
- **自己的进程**整行高亮;命令自动瘦身(去解释器路径、家目录折叠 `~`)。

## 按键

| 键 | 作用 |
|----|------|
| `c` | 切到 / 切回 **CPU 单页**(只看 CPU 进程,铺满高度,多一列 `SWAP` 定位谁在占交换) |
| `r` | 反序(**仅 CPU 单页**:CPU% 降序 ⇄ 升序) |
| `p` | **定格 / 继续**(冻结画面与时钟,显示 `⏸ 已暂停`) |
| `q` / `Ctrl-C` | 退出 |

## 安装

### 下载预编译二进制(推荐)

到 [Releases](https://github.com/Passion4ever/wcn/releases) 下载 `wcn`,然后:

```bash
chmod +x wcn
mv wcn ~/.local/bin/        # 确保 ~/.local/bin 在 PATH 里
wcn
```

静态链接,任何 x86_64 Linux 直接跑,无需额外依赖。运行时需要 `nvidia-smi`(看 GPU)、`ps`。

### 从源码构建

需要 Rust 工具链:

```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
# 产物:target/x86_64-unknown-linux-musl/release/wcn
```

> 若在受限网络/代理后构建,给 cargo 配置代理即可(`~/.cargo/config.toml` 里设 `[http] proxy = "..."`)。

## 自适应

- **宽度**:窄了系统面板堆到上方、GPU 概览的显存条自动省略、命令列按宽省略(`…`);
  小到放不下时显示"窗口太小,建议至少 …"。
- **高度**:CPU 进程行数按剩余高度伸缩,放不下不显示,提示行始终保留。
- **主题**:配色相对终端主题(`dim` + 语义色 + 反色),深/浅/任意配色都自动协调。

## 测试

```bash
cargo test    # 纯解析函数单测 + 渲染冒烟
```

## 许可

个人自用,MIT。
