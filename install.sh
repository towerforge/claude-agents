#!/bin/sh
# claude-agents installer
#
# Installs claude-agents and, run again, upgrades it: the tool does not update
# itself, re-running this script IS the update.
#
# Usage (latest):
#   curl -fsSL https://raw.githubusercontent.com/towerforge/claude-agents/main/install.sh | sh
#
# Env overrides:
#   CLAUDE_AGENTS_VERSION=0.2.0         install a specific version
#   CLAUDE_AGENTS_INSTALL_DIR=/opt/bin  install to a custom directory
#   CLAUDE_AGENTS_VARIANT=musl          force the static musl binary on Linux (no glibc dep)
#   CLAUDE_AGENTS_VARIANT=gnu           force the glibc binary on Linux
#   NO_COLOR=1                          no colour, whatever the terminal says

set -e

REPO="towerforge/claude-agents"
BINARY="claude-agents"
GITHUB_API="https://api.github.com/repos/${REPO}"
GITHUB_RELEASES="https://github.com/${REPO}/releases/download"
INSTALL_URL="https://raw.githubusercontent.com/${REPO}/main/install.sh"

# ── colours ──────────────────────────────────────────────────────────────────
# Plain ANSI, only when the output is a terminal and NO_COLOR is not set.

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ] && [ "${TERM:-}" != "dumb" ]; then
  BOLD='\033[1m'; DIM='\033[2m'; RESET='\033[0m'
  GREEN='\033[32m'; RED='\033[31m'; YELLOW='\033[33m'
else
  BOLD=''; DIM=''; RESET=''
  GREEN=''; RED=''; YELLOW=''
fi

# Column the descriptions of a command list line up at.
CMD_W=30

section() { printf "\n  ${BOLD}%s${RESET}\n" "$*"; }
kv()      { printf "     ${DIM}%-13s${RESET}%b\n" "$1" "$2"; }
note()    { printf "     ${DIM}%s${RESET}\n" "$*"; }
cmd()     { printf "     %-${CMD_W}s${DIM}%s${RESET}\n" "$1" "$2"; }
ok()      { printf "     ${GREEN}✓${RESET} %b\n" "$*"; }
warn()    { printf "     ${YELLOW}!${RESET} %s\n" "$*"; }
die()     { printf "\n  ${RED}✗${RESET} %s\n\n" "$*" >&2; exit 1; }

# ── requirements ─────────────────────────────────────────────────────────────

need() { command -v "$1" >/dev/null 2>&1 || die "Required tool not found: $1"; }
need curl
need tar

# ── platform detection ───────────────────────────────────────────────────────

detect_os() {
  case "$(uname -s)" in
    Linux)  echo linux ;;
    Darwin) echo macos ;;
    MINGW*|MSYS*|CYGWIN*)
            die "Windows is not supported: claude-agents reads running sessions with ps. Use it from WSL (Linux)." ;;
    *)      die "Unsupported OS: $(uname -s). claude-agents runs on Linux and macOS." ;;
  esac
}

detect_arch() {
  case "$(uname -m)" in
    x86_64|amd64)  echo x86_64  ;;
    aarch64|arm64) echo aarch64 ;;
    *)             die "No prebuilt binary for $(uname -m). Build from source: cargo install --git https://github.com/${REPO}" ;;
  esac
}

detect_variant() {
  if ldd --version 2>&1 | grep -qi musl; then
    echo musl
  else
    echo gnu
  fi
}

# ── package name ─────────────────────────────────────────────────────────────

build_package_name() {
  _os="$1"; _arch="$2"; _variant="$3"
  if [ "$_os" = "linux" ] && [ "$_variant" = "musl" ]; then
    echo "${BINARY}-${_os}-${_arch}-musl.tar.gz"
  else
    echo "${BINARY}-${_os}-${_arch}.tar.gz"
  fi
}

# ── github helpers ───────────────────────────────────────────────────────────

fetch_latest_version() {
  curl -fsSL "${GITHUB_API}/releases/latest" \
    | grep '"tag_name"' \
    | sed 's/.*"tag_name": *"v\([^"]*\)".*/\1/'
}

# ── checksum verification ────────────────────────────────────────────────────

verify_checksum() {
  _file="$1"; _sums="$2"
  _name=$(basename "$_file")
  _expected=$(grep " ${_name}$" "$_sums" 2>/dev/null | awk '{print $1}')

  if [ -z "$_expected" ]; then
    warn "no checksum entry for ${_name}: not verified"
    return 0
  fi

  if command -v sha256sum >/dev/null 2>&1; then
    _actual=$(sha256sum "$_file" | awk '{print $1}')
  elif command -v shasum >/dev/null 2>&1; then
    _actual=$(shasum -a 256 "$_file" | awk '{print $1}')
  else
    warn "sha256sum / shasum not found: not verified"
    return 0
  fi

  [ "$_actual" = "$_expected" ] \
    || die "Checksum mismatch!
  expected: ${_expected}
  got:      ${_actual}"

  ok "sha-256 verified"
}

# ── install directory ─────────────────────────────────────────────────────────

default_install_dir() {
  if [ "$(id -u)" = "0" ]; then
    echo /usr/local/bin
  else
    echo "${HOME}/.local/bin"
  fi
}

# ─────────────────────────────────────────────────────────────────────────────
# MAIN
# ─────────────────────────────────────────────────────────────────────────────

printf "\n  ${BOLD}claude-agents${RESET}  ${DIM}installer · github.com/${REPO}${RESET}\n"
printf "  ${DIM}lists your Claude Code subagents and marks the ones in use${RESET}\n"

# ── step 1: detect platform ──────────────────────────────────────────────────

section "1 · platform"

OS=$(detect_os)
ARCH=$(detect_arch)

VARIANT=""
if [ "$OS" = "linux" ]; then
  if [ -n "${CLAUDE_AGENTS_VARIANT:-}" ]; then
    case "$CLAUDE_AGENTS_VARIANT" in
      musl|gnu) VARIANT="$CLAUDE_AGENTS_VARIANT" ;;
      *) die "CLAUDE_AGENTS_VARIANT must be musl or gnu, not '${CLAUDE_AGENTS_VARIANT}'" ;;
    esac
  else
    VARIANT=$(detect_variant)
  fi
fi

kv "os" "$OS"
kv "arch" "$ARCH"
if [ -n "$VARIANT" ]; then
  if [ -n "${CLAUDE_AGENTS_VARIANT:-}" ]; then
    kv "libc" "${VARIANT}  ${DIM}(CLAUDE_AGENTS_VARIANT)${RESET}"
  else
    kv "libc" "$VARIANT"
  fi
fi

# ── step 2: resolve version ──────────────────────────────────────────────────

section "2 · release"

INSTALL_DIR="${CLAUDE_AGENTS_INSTALL_DIR:-$(default_install_dir)}"

# An existing installation: `claude-agents --version` prints
# "claude-agents 0.1.0", the version is the last word.
CURRENT_VERSION=""
for _candidate in "${INSTALL_DIR}/${BINARY}" "$(command -v ${BINARY} 2>/dev/null || true)"; do
  [ -z "$_candidate" ] && continue
  if [ -x "$_candidate" ]; then
    CURRENT_VERSION=$("$_candidate" --version 2>/dev/null | awk '{print $NF}' || true)
    break
  fi
done

VERSION="${CLAUDE_AGENTS_VERSION:-}"
VERSION="${VERSION#v}"
if [ -z "$VERSION" ]; then
  note "asking github for the latest release…"
  VERSION=$(fetch_latest_version) || die "Could not fetch the latest version from GitHub"
  [ -n "$VERSION" ] || die "No release found at https://github.com/${REPO}/releases"
fi

PACKAGE=$(build_package_name "$OS" "$ARCH" "$VARIANT")
URL="${GITHUB_RELEASES}/v${VERSION}/${PACKAGE}"
CHECKSUMS_URL="${GITHUB_RELEASES}/v${VERSION}/checksums.txt"

if [ -z "$CURRENT_VERSION" ]; then
  MODE="install"
  kv "version" "v${VERSION}"
elif [ "$CURRENT_VERSION" = "$VERSION" ]; then
  MODE="reinstall"
  kv "version" "v${VERSION}  ${DIM}(already installed)${RESET}"
else
  MODE="upgrade"
  kv "version" "${DIM}v${CURRENT_VERSION}${RESET}  →  ${BOLD}v${VERSION}${RESET}"
fi
kv "package" "$PACKAGE"
kv "install to" "${INSTALL_DIR}/${BINARY}"

# ── confirmation ─────────────────────────────────────────────────────────────
# Piped into sh, stdin is the script: the answer is read from the terminal.

if [ -t 0 ] || { [ -c /dev/tty ] && (: </dev/tty) 2>/dev/null; }; then
  printf "\n"
  if [ "$MODE" = "reinstall" ]; then
    printf "     ${DIM}already at v${VERSION} · reinstall?${RESET} [y/N] "
    read -r _reply </dev/tty
    case "$_reply" in
      [yY]*) ;;
      *) printf "\n     ${DIM}nothing was touched${RESET}\n\n"; exit 0 ;;
    esac
  else
    _action="install"
    [ "$MODE" = "upgrade" ] && _action="upgrade"
    printf "     ${DIM}press${RESET} enter ${DIM}to ${_action}, or${RESET} ctrl+c ${DIM}to cancel${RESET} "
    read -r _ </dev/tty
  fi
fi

# ── step 3: download & verify ────────────────────────────────────────────────

section "3 · download"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT TERM

note "${URL}"
curl -fL --progress-bar -o "${TMP}/${PACKAGE}" "$URL" \
  || die "Download failed. Check that release v${VERSION} has the asset ${PACKAGE}:
  https://github.com/${REPO}/releases/tag/v${VERSION}"

if curl -fsSL -o "${TMP}/checksums.txt" "$CHECKSUMS_URL" 2>/dev/null; then
  verify_checksum "${TMP}/${PACKAGE}" "${TMP}/checksums.txt"
else
  warn "checksums.txt not published for v${VERSION}: not verified"
fi

# ── step 4: install ──────────────────────────────────────────────────────────

section "4 · install"

tar -xzf "${TMP}/${PACKAGE}" -C "$TMP"
[ -f "${TMP}/${BINARY}" ] || die "Binary '${BINARY}' not found inside the archive"

mkdir -p "$INSTALL_DIR"
install -m 755 "${TMP}/${BINARY}" "${INSTALL_DIR}/${BINARY}"

case "$MODE" in
  upgrade)   ok "${INSTALL_DIR}/${BINARY}  ${DIM}(v${CURRENT_VERSION} → v${VERSION})${RESET}" ;;
  reinstall) ok "${INSTALL_DIR}/${BINARY}  ${DIM}(reinstalled v${VERSION})${RESET}" ;;
  *)         ok "${INSTALL_DIR}/${BINARY}  ${DIM}(v${VERSION})${RESET}" ;;
esac

# PATH hint
case ":${PATH}:" in
  *":${INSTALL_DIR}:"*) ;;
  *)
    warn "${INSTALL_DIR} is not in your PATH"
    note "add to your shell profile:  export PATH=\"${INSTALL_DIR}:\$PATH\""
    ;;
esac

# ── done ─────────────────────────────────────────────────────────────────────

section "ready"

cmd "claude-agents" "interactive screen (agents used in the last 10 min)"
cmd "claude-agents -m 60" "widen the active window to 60 minutes"
cmd "claude-agents --plain" "print the table and exit"
cmd "claude-agents --plain -v" "the table plus open Claude sessions"
cmd "claude-agents -r ~/code" "search ~/code instead of your home (repeatable)"
cmd "claude-agents --help" "every flag"
printf "\n"
note "to upgrade later, run the installer again:"
note "curl -fsSL ${INSTALL_URL} | sh"
printf "\n"
