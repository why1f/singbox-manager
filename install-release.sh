#!/usr/bin/env bash
# 从 GitHub Release 下载预编译二进制并安装。无需 Rust/gcc 等编译依赖。
#
# 用法：
#   sudo REPO=youruser/singbox-manager ./install-release.sh             # 安装最新 release
#   sudo REPO=youruser/singbox-manager VERSION=v0.1.0 ./install-release.sh  # 指定版本

set -euo pipefail

REPO="${REPO:-}"
VERSION="${VERSION:-latest}"
PREFIX="/usr/local"
BIN_PATH="$PREFIX/bin/sb"
CONFIG_DIR="/etc/sing-box-manager"
CONFIG_PATH="$CONFIG_DIR/config.toml"
DATA_DIR="/var/lib/sing-box-manager"
SERVICE_PATH="/etc/systemd/system/sb-manager.service"

fail() { printf '错误: %s\n' "$1" >&2; exit 1; }
note() { printf '%s\n' "$1"; }

[ "${EUID:-$(id -u)}" -eq 0 ] || fail "请使用 root 运行"
[ "$(uname -s)" = "Linux" ] || fail "仅支持 Linux"
[ -n "$REPO" ] || fail "请设置 REPO 环境变量，例如：REPO=youruser/singbox-manager"
command -v systemctl >/dev/null 2>&1 || fail "未检测到 systemctl"
command -v curl >/dev/null 2>&1 || fail "需要 curl"
command -v tar >/dev/null 2>&1 || fail "需要 tar"

case "$(uname -m)" in
  x86_64)         TARGET="x86_64-unknown-linux-musl" ;;
  aarch64|arm64)  TARGET="aarch64-unknown-linux-musl" ;;
  *) fail "不支持的架构: $(uname -m)" ;;
esac

if [ "$VERSION" = "latest" ]; then
  VERSION="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n1)"
  [ -n "$VERSION" ] || fail "无法获取最新 release tag"
fi

ASSET="sb-${VERSION}-${TARGET}.tar.gz"
URL="https://github.com/${REPO}/releases/download/${VERSION}/${ASSET}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

note "下载 $URL"
curl -fL --retry 3 -o "$TMP/$ASSET" "$URL"
curl -fL --retry 3 -o "$TMP/$ASSET.sha256" "$URL.sha256" || true
if [ -s "$TMP/$ASSET.sha256" ]; then
  ( cd "$TMP" && sha256sum -c "$ASSET.sha256" ) || fail "校验失败"
fi

tar xzf "$TMP/$ASSET" -C "$TMP"
SRC_DIR="$TMP/sb-${VERSION}-${TARGET}"
[ -x "$SRC_DIR/sb" ] || fail "包内未找到可执行 sb"

install -d "$CONFIG_DIR" "$DATA_DIR" "$PREFIX/bin"
install -m 0755 "$SRC_DIR/sb" "$BIN_PATH"
[ -f "$CONFIG_PATH" ] || install -m 0644 "$SRC_DIR/config.toml" "$CONFIG_PATH"
install -m 0644 "$SRC_DIR/sb-manager.service" "$SERVICE_PATH"

# 探测 sing-box，并写入 config.toml
SB_BIN=""
if [ -x /usr/local/bin/sing-box ]; then SB_BIN=/usr/local/bin/sing-box
elif command -v sing-box >/dev/null 2>&1; then SB_BIN="$(command -v sing-box)"
fi
SB_CONFIG=""
for c in /etc/sing-box/config.json /usr/local/etc/sing-box/config.json; do
  [ -f "$c" ] && { SB_CONFIG="$c"; break; }
done
if [ -n "$SB_BIN" ] && [ -n "$SB_CONFIG" ]; then
  python3 - "$CONFIG_PATH" "$SB_BIN" "$SB_CONFIG" <<'PY' 2>/dev/null || true
import sys, re, pathlib
path, binp, cfgp = sys.argv[1:]
p = pathlib.Path(path); text = p.read_text() if p.exists() else ""
def setk(t, k, v):
    v = v.replace('\\','\\\\').replace('"','\\"')
    pat = re.compile(r'(?m)^(\s*'+re.escape(k)+r'\s*=\s*).*$')
    return pat.sub(lambda m: f'{m.group(1)}"{v}"', t) if pat.search(t) else t + f'\n{k} = "{v}"\n'
text = setk(text, 'binary_path', binp)
text = setk(text, 'config_path', cfgp)
p.write_text(text)
PY
fi

systemctl daemon-reload
systemctl enable sb-manager.service

note ""
note "安装完成。版本：$VERSION  目标：$TARGET"
note "启动："
note "  systemctl start sb-manager.service"
note "  systemctl status sb-manager.service"
note "  journalctl -u sb-manager -f"
