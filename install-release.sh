#!/usr/bin/env bash
# 从 GitHub Release 下载预编译二进制并安装。无需 Rust/gcc 等编译依赖。
#
# 一键:
#   curl -fsSL https://raw.githubusercontent.com/why1f/singbox-manager/master/install-release.sh | sudo bash
#
# 进阶:
#   REPO=你的用户名/singbox-manager VERSION=v0.2.1 bash install-release.sh

set -euo pipefail

REPO="${REPO:-why1f/singbox-manager}"
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

# 若已安装且与目标版本相同，默认跳过（FORCE=1 可强制重装）
if [ -x "$BIN_PATH" ] && [ "${FORCE:-0}" != "1" ]; then
  CURRENT_VER="$("$BIN_PATH" --version 2>/dev/null | awk '{print $2}')"
  if [ -n "$CURRENT_VER" ]; then
    TARGET_VER="${VERSION#v}"
    if [ "$CURRENT_VER" = "$TARGET_VER" ]; then
      note "已是目标版本 $VERSION，跳过下载与安装（FORCE=1 可强制重装）"
      # 但仍尝试启动服务（首次安装后用户可能忘了 start）
      if systemctl is-enabled --quiet sb-manager.service 2>/dev/null \
         && ! systemctl is-active --quiet sb-manager.service; then
        note "服务未运行，尝试启动"
        systemctl start sb-manager.service || true
      fi
      exit 0
    else
      note "升级 $CURRENT_VER → ${VERSION#v}"
    fi
  fi
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

# —— 真正把文件放到最终位置 ——
install -d "$CONFIG_DIR" "$DATA_DIR" "$PREFIX/bin"
install -m 0755 "$SRC_DIR/sb" "$BIN_PATH"
[ -f "$CONFIG_PATH" ] || install -m 0644 "$SRC_DIR/config.toml" "$CONFIG_PATH"
install -m 0644 "$SRC_DIR/sb-manager.service" "$SERVICE_PATH"

# 探测 sing-box（不强制安装；缺失时进 TUI 内核页安装）
SB_BIN=""
if [ -x /usr/local/bin/sing-box ]; then SB_BIN=/usr/local/bin/sing-box
elif command -v sing-box >/dev/null 2>&1; then SB_BIN="$(command -v sing-box)"
fi

SB_CONFIG=""
for c in /etc/sing-box/config.json /usr/local/etc/sing-box/config.json; do
  [ -f "$c" ] && { SB_CONFIG="$c"; break; }
done

# 写入 config.toml
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

# 清除可能的 stale alias
cat > /etc/profile.d/sb-manager.sh <<'EOF'
unalias sb 2>/dev/null || true
unalias sing-box 2>/dev/null || true
EOF
chmod 644 /etc/profile.d/sb-manager.sh
for rc in /root/.bashrc /root/.bash_aliases; do
  [ -f "$rc" ] && sed -i '/^[[:space:]]*alias[[:space:]]\+\(sb\|sing-box\)=/d' "$rc" 2>/dev/null || true
done

systemctl daemon-reload
systemctl enable sb-manager.service
ln -sf "$BIN_PATH" /usr/bin/sb 2>/dev/null || true
hash -r 2>/dev/null || true

# 升级场景：如果服务已经在跑，新二进制需要重启；初次安装也启动起来
if systemctl is-active --quiet sb-manager.service; then
  note "检测到 sb-manager 正在运行，重启以加载新版本"
  systemctl restart sb-manager.service
else
  note "启动 sb-manager"
  systemctl start sb-manager.service || note "(启动失败，请手动 systemctl status sb-manager)"
fi

note ""
note "安装完成。版本：$VERSION  目标：$TARGET"
note "常用命令:"
note "  sb                              # 进 TUI"
note "  systemctl status sb-manager     # 看服务"
note "  journalctl -u sb-manager -f     # 看日志"
note ""
note "若 sb 报找不到：unalias sb sing-box 2>/dev/null; hash -r  或重新登录"
[ -z "$SB_BIN" ] && note "sing-box 未安装 — 进 TUI (sb) 到内核[5]页按 v 一键装带 v2ray_api 的版本。"
