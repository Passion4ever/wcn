#!/usr/bin/env bash
# wcn 安装 / 更新脚本 —— 装和更新是同一条命令,想更新就再跑一遍。
#
#   curl -fsSL https://raw.githubusercontent.com/Passion4ever/wcn/main/install.sh | bash
#
# 需要走代理时(curl 会自动读这个环境变量):
#   curl -fsSL https://raw.githubusercontent.com/Passion4ever/wcn/main/install.sh \
#     | HTTPS_PROXY=socks5://user:pass@127.0.0.1:PORT bash

set -euo pipefail

REPO="Passion4ever/wcn"
URL="https://github.com/$REPO/releases/latest/download/wcn"
LOCAL_DIR="$HOME/.local/bin"
GLOBAL_DIR="/usr/local/bin"

# 注:管道里一律不用 head —— 它读够就退出会给上游发 SIGPIPE,
# 在 pipefail + set -e 下会让脚本静默暴毙(且是竞态,时灵时不灵)。用 awk 读完全部输入。

# --- 1. 环境检查:glibc 是硬门槛,没驱动只是 GPU 面板空着 ---------------------
GLIBC="$(ldd --version 2>/dev/null | awk 'NR==1{print $NF}')"
if [ -n "$GLIBC" ] && [ "$(printf '%s\n2.17\n' "$GLIBC" | sort -V | awk 'NR==1{print}')" != "2.17" ]; then
  echo "✗ 本机 glibc $GLIBC < 2.17,预编译二进制跑不起来。" >&2
  echo "  可在本机自行编译:https://github.com/$REPO" >&2
  exit 1
fi
if ! ldconfig -p 2>/dev/null | grep -q libnvidia-ml && [ ! -e /proc/driver/nvidia/version ]; then
  echo "! 没检测到 NVIDIA 驱动:GPU 面板会是空的,系统面板照常。"
fi

# --- 2. 版本:先查清楚,好告诉用户是装还是更新 -------------------------------
# 查版本必须用 setsid + timeout:旧版不认 --version 会直接开 TUI,而且 crossterm 会绕过
# stdout 去抢 /dev/tty —— 那样不仅卡住,被 SIGTERM 杀掉时 Drop 不执行,会把你的终端留在
# raw 模式 + 备用屏。setsid 剥离控制终端,它打不开 /dev/tty,只能立刻报错退出。
ver_of() { setsid timeout 5 "$1" --version 2>/dev/null </dev/null | awk '{print $2}' || true; }

# alias 优先级高于 PATH,会完全盖住刚装的命令。脚本是子进程,删不掉父 shell 内存里的
# alias(试过无效),只能检测 + 帮改配置文件;无论如何都得 source 或重开终端才生效。
# 注意:即使"已是最新无需更新"也必须查 —— 那正是最让人懵的场景(装了却还是旧的)。
check_alias() {
  local out files f ans
  out="$("${SHELL:-bash}" -ic 'type wcn' 2>/dev/null | grep -i alias || true)"
  case "$out" in
    *wcn*) ;;      # bash: "wcn is aliased to ..." / zsh: "wcn is an alias for ..."
    *) return 0 ;;
  esac

  echo ""
  echo "! 你的 shell 里 wcn 是一个 alias,它会盖过刚装的命令:"
  echo "    $out"

  files=""
  for f in "$HOME/.bashrc" "$HOME/.bash_aliases" "$HOME/.zshrc" "$HOME/.bash_profile" "$HOME/.profile"; do
    [ -f "$f" ] || continue
    grep -qE '^[[:space:]]*alias[[:space:]]+wcn=' "$f" 2>/dev/null && files="$files $f"
  done

  if [ -z "$files" ]; then
    echo "  没找到它定义在哪个配置文件,请自行删除后重开终端。"
    echo "  临时绕过:command wcn   或   \\wcn"
    return 0
  fi
  echo "  定义在:$files"

  if ! : 2>/dev/null < /dev/tty; then
    echo "  (非交互,未自动处理)删掉该行后重开终端;临时绕过:command wcn"
    return 0
  fi

  printf "  要帮你把这行注释掉吗? [y/N]: " > /dev/tty
  read -r ans < /dev/tty || ans=""
  echo "" > /dev/tty
  case "$ans" in
    y|Y|yes|YES)
      for f in $files; do
        sed -i -E 's/^([[:space:]]*alias[[:space:]]+wcn=)/# \1/' "$f"
        echo "  ✓ 已注释掉 $f 里的 alias"
      done
      echo "  → 执行 source ~/.bashrc(或重开终端)后,wcn 才会指向新命令。"
      ;;
    *)
      echo "  已保留 alias。临时绕过:command wcn   或   \\wcn"
      ;;
  esac
}

# 要比对的是"将被替换的那个 wcn":`wcn update` 传了 WCN_DIR 就以它为准,
# 否则才回退到 PATH 查找 —— 否则 PATH 里若有另一个更新的 wcn,会误判"已最新"而漏更目标。
if [ -n "${WCN_DIR:-}" ]; then
  TARGET="$WCN_DIR/wcn"                              # 指定了目录,目标就是它,不看 PATH
else
  TARGET="$(command -v wcn 2>/dev/null || true)"
fi
CUR=""
[ -x "$TARGET" ] && CUR="$(ver_of "$TARGET")"        # 取不到版本就当未知,继续装
LATEST="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null \
  | sed -nE 's/.*"tag_name": *"v?([^"]+)".*/\1/p' | awk 'NR==1{print}' || true)"

if [ -n "$CUR" ] && [ -n "$LATEST" ] && [ "$CUR" = "$LATEST" ]; then
  echo "✓ 已是最新版 $CUR($TARGET),无需更新。"
  check_alias      # 已最新也要查:装了却敲不出来,正是最让人懵的情况
  exit 0
fi

if [ -n "$CUR" ]; then
  echo "发现新版本:当前 $CUR  →  最新 ${LATEST:-未知}"
elif [ -n "$TARGET" ]; then
  echo "已装 wcn(版本未知,旧版无 --version)  →  将更新到 ${LATEST:-最新}"
else
  echo "将安装 wcn ${LATEST:+v$LATEST}"
fi

# --- 3. 装哪儿:让用户选;没 tty(非交互)则默认本地,不卡住 -----------------
choose_dir() {
  # WCN_DIR:由 `wcn update` 传入,表示"就地更新到现有安装位置",不必再问
  if [ -n "${WCN_DIR:-}" ]; then
    echo "$WCN_DIR"; return
  fi
  # curl|bash 时 stdin 是脚本,要提问必须走 /dev/tty。
  # 两个坑:① 不能用 [ -r /dev/tty ] —— 它只看权限位(对所有人可读),没有控制终端时
  # 照样返回真,真正 open() 才失败,必须实际试开一次;② 2>/dev/null 必须写在
  # < /dev/tty 之前 —— 重定向按顺序处理,写在后面时报错已经打到终端上了。
  if ! : 2>/dev/null < /dev/tty; then   # 非交互:不提问,默认本地
    echo "$LOCAL_DIR"; return
  fi
  {
    echo ""
    echo "装到哪里?"
    echo "  1) $GLOBAL_DIR      全局,所有用户可用(需 sudo)"
    echo "  2) $LOCAL_DIR   仅当前用户(默认,免 sudo)"
    printf "请选择 [1/2] (回车默认 2): "
  } > /dev/tty
  local ans=""
  read -r ans < /dev/tty || ans=""
  echo "" > /dev/tty
  case "$ans" in
    1) echo "$GLOBAL_DIR" ;;
    *) echo "$LOCAL_DIR" ;;
  esac
}

DIR="$(choose_dir)"
SUDO=""

# 选了全局但没写权限:先试 sudo,sudo 也不行就退回本地
if [ "$DIR" = "$GLOBAL_DIR" ] && [ ! -w "$GLOBAL_DIR" ]; then
  # 注意别屏蔽 sudo 的 stderr —— 密码提示走的就是 stderr,屏了用户会以为卡死。
  if command -v sudo >/dev/null 2>&1 && sudo -v </dev/tty; then
    SUDO="sudo"
  else
    echo "! 没有 $GLOBAL_DIR 的写权限,sudo 也用不了 —— 退回 $LOCAL_DIR"
    DIR="$LOCAL_DIR"
  fi
fi
[ -n "$SUDO" ] || mkdir -p "$DIR"

# --- 4. 下载 + 原子替换:同目录换名落盘再 mv,正在运行也能换 ------------------
TMP="$(mktemp)"
STAGE="$DIR/.wcn.new.$$"
trap 'rm -f "$TMP"; [ -n "${SUDO:-}" ] && $SUDO rm -f "$STAGE" 2>/dev/null || rm -f "$STAGE" 2>/dev/null; true' EXIT

echo ">> 下载中 ..."
curl -fsSL "$URL" -o "$TMP"
$SUDO cp "$TMP" "$STAGE"          # 落到目标目录、用不同文件名 → 不会 Text file busy
$SUDO chmod 755 "$STAGE"
$SUDO mv -f "$STAGE" "$DIR/wcn"   # rename 原子替换,且不打断已开着的窗口
trap - EXIT
rm -f "$TMP"

# --- 5. 收尾 ----------------------------------------------------------------
NEW="$(ver_of "$DIR/wcn")"          # 装的若是旧版(无 --version)取不到,就退回用 tag 显示
SHOWN="${NEW:-${LATEST:-未知}}"
if [ -n "$CUR" ]; then
  echo "✓ 更新完成:$CUR  →  $SHOWN   ($DIR/wcn)"
else
  echo "✓ 安装完成:$SHOWN   ($DIR/wcn)"
fi

case ":$PATH:" in
  *":$DIR:"*) echo "  直接敲 wcn 即可。" ;;
  *)
    echo "! $DIR 不在 PATH 中,加这行到 ~/.bashrc 后重开终端:"
    echo "    export PATH=\"$DIR:\$PATH\""
    ;;
esac

# --- 6. alias 冲突检测(函数定义在前面,装完 / 已最新 两条路径都会走) ---------
check_alias
