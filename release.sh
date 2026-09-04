#!/usr/bin/env bash
# 发布流程:用 cargo-zigbuild 编「到处能跑」的 glibc 2.17 二进制,再建 GitHub release。
#
# 前置:
#   - Rust 工具链 + cargo-zigbuild(cargo install cargo-zigbuild)
#   - zig 在 PATH(https://ziglang.org/download)
#   - gh 已登录(gh auth login)
# 版本号自动取自 Cargo.toml。发布说明:有 RELEASE_NOTES.md 就用它,否则自动生成。
#
# 受限网络/代理后使用(不要把代理密钥写进本脚本或仓库):
#   HTTPS_PROXY=socks5://user:pass@127.0.0.1:PORT ./release.sh

set -euo pipefail
cd "$(dirname "$0")"

TARGET="x86_64-unknown-linux-gnu.2.17"   # 低到 glibc 2.17(CentOS7/Ubuntu14.04+)
BIN="target/x86_64-unknown-linux-gnu/release/wcn"
# 资产名带上平台/架构:release 页面一眼看清这份二进制是给谁编的。不带版本号 —— tag 已经
# 写了一遍,而且名字固定不变,latest/download/<名字> 这个免查 API 的下载入口才保得住。
ASSET="$(dirname "$BIN")/wcn-x86_64-linux-gnu"
VERSION="v$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"(.*)".*/\1/')"

echo ">> 构建 $TARGET ..."
cargo zigbuild --release --target "$TARGET"

FLOOR="$(objdump -T "$BIN" | grep -oP 'GLIBC_\K[0-9.]+' | sort -V | tail -1)"
echo ">> 产物 $BIN,glibc 门槛 $FLOOR"
cp "$BIN" "$ASSET"        # gh 拿文件名当资产名,所以换个名字再传

echo ">> 建 release $VERSION ..."
if [ -f RELEASE_NOTES.md ]; then
  gh release create "$VERSION" "$ASSET" --title "$VERSION" --notes-file RELEASE_NOTES.md
else
  gh release create "$VERSION" "$ASSET" --title "$VERSION" --generate-notes
fi

echo ">> 完成:$VERSION 已发布,资产 $(basename "$ASSET")(glibc $FLOOR)"
