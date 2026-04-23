#!/usr/bin/env bash
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PREFIX="/usr/local"
BIN_PATH="/etc/sing-box/bin/sb"
CONFIG_DIR="/etc/sing-box/manager"
CONFIG_PATH="$CONFIG_DIR/config.toml"
DATA_DIR="/etc/sing-box/manager"
SERVICE_PATH="/etc/systemd/system/sb-manager.service"
SB_BIN=""
SB_CONFIG=""

fail() { printf '错误: %s\n' "$1" >&2; exit 1; }
note() { printf '%s\n' "$1"; }

require_root() {
  [ "${EUID:-$(id -u)}" -eq 0 ] || fail "请使用 root 运行 install.sh"
}

require_linux() {
  [ "$(uname -s)" = "Linux" ] || fail "install.sh 仅支持 Linux 服务器环境"
}

require_systemd() {
  command -v systemctl >/dev/null 2>&1 || fail "未检测到 systemctl"
}

install_packages() {
  # rustls 路线不再需要 libssl-dev
  if command -v apt-get >/dev/null 2>&1; then
    export DEBIAN_FRONTEND=noninteractive
    apt-get update
    apt-get install -y build-essential pkg-config curl ca-certificates
  elif command -v dnf >/dev/null 2>&1; then
    dnf install -y gcc make pkgconfig curl ca-certificates
  elif command -v yum >/dev/null 2>&1; then
    yum install -y gcc make pkgconfig curl ca-certificates
  elif command -v pacman >/dev/null 2>&1; then
    pacman -Sy --noconfirm base-devel curl ca-certificates
  elif command -v apk >/dev/null 2>&1; then
    apk add --no-cache build-base pkgconf curl ca-certificates
  else
    note "未识别的包管理器，跳过依赖安装（请确保 gcc/make/pkg-config/curl 已安装）"
  fi
}

ensure_rust() {
  if ! command -v cargo >/dev/null 2>&1; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
  fi
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
}

build_project() {
  cargo build --manifest-path "$PROJECT_DIR/Cargo.toml" --release
}

detect_singbox_binary() {
  SB_BIN="/etc/sing-box/bin/sing-box"
  if [ ! -x "$SB_BIN" ] && [ -x /usr/local/bin/sing-box ]; then
    SB_BIN="/usr/local/bin/sing-box"; return
  fi
  if command -v sing-box >/dev/null 2>&1; then
    SB_BIN="$(command -v sing-box)"; return
  fi
  note "提示: 未检测到 sing-box。装完 sb-manager 后可进 TUI 的「内核」页一键安装。"
  SB_BIN=""
}

detect_singbox_config() {
  for candidate in /etc/sing-box/config.json; do
    if [ -f "$candidate" ]; then SB_CONFIG="$candidate"; return; fi
  done
  SB_CONFIG=""
}

install_files() {
  install -d "$CONFIG_DIR" "$DATA_DIR" "/etc/sing-box/bin" "/etc/sing-box/certs" "/etc/sing-box/backup"
  install -m 0755 "$PROJECT_DIR/target/release/sb" "$BIN_PATH"
  
  # 强制将现有 config.toml 的 binary_path 指向 /etc/sing-box/bin/sing-box（如果是 v0.4.0 之前的默认路径）
  if [ -f "$CONFIG_PATH" ]; then
    sed -i 's|binary_path = "/usr/local/bin/sing-box"|binary_path = "/etc/sing-box/bin/sing-box"|g' "$CONFIG_PATH"
  else
    install -m 0644 "$PROJECT_DIR/config.toml" "$CONFIG_PATH"
  fi
  
  install -m 0644 "$PROJECT_DIR/sb-manager.service" "$SERVICE_PATH"
  sed -i "s|/usr/local/bin/sb|/etc/sing-box/bin/sb|g" "$SERVICE_PATH"
}

# 用 sb 自身的原子重写代替 sed：生成最小覆盖配置，让下次启动采用新路径
patch_config() {
  [ -n "$SB_BIN" ] || return 0
  [ -n "$SB_CONFIG" ] || return 0
  python3 - "$CONFIG_PATH" "$SB_BIN" "$SB_CONFIG" <<'PY' 2>/dev/null || patch_config_sed
import sys, re, pathlib
path, binp, cfgp = sys.argv[1:]
p = pathlib.Path(path)
text = p.read_text() if p.exists() else ""
def set_kv(t, k, v):
    v = v.replace('\\', '\\\\').replace('"', '\\"')
    pat = re.compile(r'(?m)^(\s*'+re.escape(k)+r'\s*=\s*).*$')
    if pat.search(t):
        return pat.sub(lambda m: f'{m.group(1)}"{v}"', t)
    return t + f'\n{k} = "{v}"\n'
text = set_kv(text, 'binary_path', binp)
text = set_kv(text, 'config_path', cfgp)
p.write_text(text)
PY
}

patch_config_sed() {
  # 退路：python3 不可用时，用 sed
  sed_set() {
    local key="$1" value="$2"
    local esc; esc="$(printf '%s' "$value" | sed 's/[\/&]/\\&/g')"
    if grep -q "^${key}[[:space:]]*=" "$CONFIG_PATH"; then
      sed -i "s|^${key}[[:space:]]*=.*$|${key} = \"${esc}\"|" "$CONFIG_PATH"
    else
      printf '%s = "%s"\n' "$key" "$value" >> "$CONFIG_PATH"
    fi
  }
  sed_set "binary_path" "$SB_BIN"
  sed_set "config_path" "$SB_CONFIG"
}

validate_singbox() {
  [ -n "$SB_BIN" ]    || { note "跳过 sing-box 二进制校验（未安装）"; return 0; }
  [ -n "$SB_CONFIG" ] || { note "跳过 sing-box 配置校验（未找到）"; return 0; }
  "$SB_BIN" version >/dev/null
  "$SB_BIN" check -c "$SB_CONFIG" >/dev/null
  if ! grep -q '"v2ray_api"' "$SB_CONFIG"; then
    note "警告: sing-box 配置中似乎未启用 experimental.v2ray_api，后台流量同步不可用。"
  fi
}

install_shell_profile() {
  cat > /etc/profile.d/sb-manager.sh <<'EOF'
# sb-manager: 清除可能的 stale alias，确保 sb / sing-box 解析到真实二进制
unalias sb 2>/dev/null || true
unalias sing-box 2>/dev/null || true
EOF
  chmod 644 /etc/profile.d/sb-manager.sh
  for rc in /root/.bashrc /root/.bash_aliases; do
    [ -f "$rc" ] && sed -i '/^[[:space:]]*alias[[:space:]]\+\(sb\|sing-box\)=/d' "$rc" 2>/dev/null || true
  done
}

reload_systemd() {
  systemctl daemon-reload
  systemctl enable sb-manager.service
  ln -sf "$BIN_PATH" /usr/local/bin/sb 2>/dev/null || true
  hash -r 2>/dev/null || true
  note ""
  note "安装完成。可执行以下命令启动服务："
  note "  systemctl start sb-manager.service"
  note "  systemctl status sb-manager.service"
  note "  journalctl -u sb-manager -f"
  note ""
  note "若当前 shell 报 'sb 找不到命令' 或指向错误路径："
  note "  unalias sb sing-box 2>/dev/null; hash -r       # 当前 shell 立即生效"
  note "  或重新登录（/etc/profile.d/sb-manager.sh 会自动清理）"
  [ -z "${SB_BIN:-}" ] && note "sing-box 未安装 — 进 TUI（sb）后到「内核[5]」页一键安装。"
}

require_root
require_linux
require_systemd
install_packages
ensure_rust
detect_singbox_binary
detect_singbox_config
build_project
install_files
patch_config
validate_singbox
install_shell_profile
reload_systemd
