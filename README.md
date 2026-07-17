# wcn

终端里的 GPU + 系统监控,`top` 与 `nvidia-smi` 的合体。单个二进制,通过 NVML 直连显卡、读 `/proc` 取系统指标,Rust + [ratatui](https://github.com/ratatui/ratatui) 实现。

## 运行要求

- **Linux x86_64 + NVIDIA 驱动**:GPU 数据经驱动自带的 NVML 库读取,机器没装驱动就只显示系统面板。
- **glibc ≥ 2.17**:即 CentOS 7 / Ubuntu 14.04 及以后,主流发行版都满足。
  用 musl 的极简系统(如 Alpine)不支持——NVML 要动态加载,做不到全静态。
- 另需 `ps`(看进程,几乎所有系统自带)。

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

## 安装 / 更新

一行搞定。**装和更新是同一条命令**,想更新就再跑一遍:

```bash
curl -fsSL https://raw.githubusercontent.com/Passion4ever/wcn/main/install.sh | bash
```

脚本会检查环境、自动选安装目录(能写 `/usr/local/bin` 就全局,否则 `~/.local/bin`)、
已是最新则跳过;更新用原子替换,`wcn` 正开着也能换。

访问 GitHub 需要代理时:

```bash
curl -fsSL https://raw.githubusercontent.com/Passion4ever/wcn/main/install.sh \
  | HTTPS_PROXY=socks5://127.0.0.1:1080 bash
```

不想跑脚本,手动装也行:

```bash
curl -fsSL https://github.com/Passion4ever/wcn/releases/latest/download/wcn -o wcn
chmod +x wcn && mv wcn ~/.local/bin/        # 或 sudo mv wcn /usr/local/bin/
```

目标机连不上 GitHub,就在能联网的机器上下载后 `scp` 过去。
预编译二进制按 glibc 2.17 链接,满足「运行要求」的主流 Linux 直接跑。查版本:`wcn --version`。

## 许可

个人自用,MIT。
