#!/bin/sh
# claude-agents installer
#
# Installs claude-agents. From then on it updates itself with
# `claude-agents update`; run again over an installation that can do that, this
# script sends you there unless CLAUDE_AGENTS_FORCE=1.
#
# Usage (latest):
#   curl -fsSL https://raw.githubusercontent.com/towerforge/claude-agents/main/install.sh | sh
#
# Env overrides:
#   CLAUDE_AGENTS_VERSION=0.2.0         install a specific version
#   CLAUDE_AGENTS_INSTALL_DIR=/opt/bin  install to a custom directory
#   CLAUDE_AGENTS_VARIANT=musl          force the static musl binary on Linux (no glibc dep)
#   CLAUDE_AGENTS_VARIANT=gnu           force the glibc binary on Linux
#   CLAUDE_AGENTS_FORCE=1               install even if claude-agents is already
#                                       there and can update itself
#   NO_COLOR=1                          no colour, whatever the terminal says

set -e

REPO="towerforge/claude-agents"
BINARY="claude-agents"
GITHUB_API="https://api.github.com/repos/${REPO}"
GITHUB_RELEASES="https://github.com/${REPO}/releases/download"
INSTALL_URL="https://raw.githubusercontent.com/${REPO}/main/install.sh"

# ── the claude-agents palette ────────────────────────────────────────────────
# The same tokens as the screen (src/tui/theme.rs): the full hex when the
# terminal announces truecolor, the xterm-256 approximation otherwise, and
# nothing at all when the output is not a terminal or NO_COLOR is set.

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ] && [ "${TERM:-}" != "dumb" ]; then
  BOLD='\033[1m'; DIM='\033[2m'; RESET='\033[0m'
  case "${COLORTERM:-}" in
    truecolor|24bit)
      CLAUDE='\033[38;2;215;119;87m'   # claude       #d77757
      SOFT='\033[38;2;235;159;127m'    # claude-soft  #eb9f7f
      INK='\033[38;2;250;249;245m'     # ink          #faf9f5
      MUTED='\033[38;2;153;153;153m'   # ink-muted    #999999
      LINE='\033[38;2;80;80;80m'       # subtle       #505050
      RED='\033[38;2;255;107;128m'     # alert        #ff6b80
      AMBER='\033[38;2;229;181;103m'   # warning      #e5b567
      ;;
    *)
      CLAUDE='\033[38;5;173m'; SOFT='\033[38;5;216m'; INK='\033[38;5;255m'
      MUTED='\033[38;5;246m'; LINE='\033[38;5;239m'; RED='\033[38;5;204m'
      AMBER='\033[38;5;179m'
      ;;
  esac
else
  BOLD=''; DIM=''; RESET=''
  CLAUDE=''; SOFT=''; INK=''; MUTED=''; LINE=''; RED=''; AMBER=''
fi

# ── the pieces every block is drawn with ─────────────────────────────────────

# Columns a rule spans, the two-space indent aside.
RULE_W=66
# Column the descriptions of a command list line up at.
CMD_W=34

_dashes() {
  _n=$1; _s=''
  while [ "$_n" -gt 0 ]; do _s="${_s}─"; _n=$((_n - 1)); done
  printf '%s' "$_s"
}

# Section heading, like the screen's: ` 2 · release ───…`, the title in claude
# over a solid rule in subtle. The number is optional. The labels are ASCII, so
# bytes count as columns.
rule() {
  if [ -n "$1" ]; then
    _label="$1 · $2"; _cols=$(( ${#1} + 3 + ${#2} ))
  else
    _label="$2"; _cols=${#2}
  fi
  _fill=$(( RULE_W - _cols - 1 ))
  [ "$_fill" -lt 0 ] && _fill=0
  printf "\n  ${CLAUDE}${BOLD}%s${RESET} ${LINE}%s${RESET}\n" \
    "$_label" "$(_dashes "$_fill")"
}

kv()   { printf "     ${MUTED}%-13s${RESET}${INK}%b${RESET}\n" "$1" "$2"; }
note() { printf "     ${MUTED}%s${RESET}\n" "$*"; }
cmd()  { printf "     ${SOFT}%-${CMD_W}s${RESET}${MUTED}%s${RESET}\n" "$1" "$2"; }
ok()   { printf "     ${CLAUDE}✓${RESET} %b\n" "$*"; }
warn() { printf "     ${AMBER}!${RESET} ${MUTED}%s${RESET}\n" "$*"; }
die()  { printf "\n  ${RED}✗${RESET} %s\n\n" "$*" >&2; exit 1; }

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

# The header block: the screen's star in claude, the name, what it does, and
# where it comes from.
printf "\n"
printf "  ${CLAUDE}✦${RESET}  ${INK}${BOLD}claude-agents${RESET}\n"
printf "     ${MUTED}lists your Claude Code subagents and marks the ones in use${RESET}\n"
printf "     ${LINE}installer · github.com/${REPO}${RESET}\n"

# ── step 1: detect platform ──────────────────────────────────────────────────

rule 1 platform

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

rule 2 release

INSTALL_DIR="${CLAUDE_AGENTS_INSTALL_DIR:-$(default_install_dir)}"

# An existing installation: `claude-agents --version` prints
# "claude-agents 0.1.0", the version is the last word.
CURRENT_VERSION=""
EXISTING_PATH=""
for _candidate in "${INSTALL_DIR}/${BINARY}" "$(command -v ${BINARY} 2>/dev/null || true)"; do
  [ -z "$_candidate" ] && continue
  if [ -x "$_candidate" ]; then
    EXISTING_PATH="$_candidate"
    CURRENT_VERSION=$("$_candidate" --version 2>/dev/null | awk '{print $NF}' || true)
    break
  fi
done

# claude-agents updates itself, so this script is for the first install.
# Whether the one that is there can do it is asked of the binary and not of its
# version number: one from before `claude-agents update` existed is upgraded
# here as always.
if [ -n "$CURRENT_VERSION" ] && [ -z "${CLAUDE_AGENTS_FORCE:-}" ] \
   && "$EXISTING_PATH" update --help >/dev/null 2>&1; then
  kv "version" "v${CURRENT_VERSION}"
  kv "path" "${EXISTING_PATH}"
  ok "claude-agents is already installed"
  printf "\n"
  printf "     ${INK}It updates itself.${RESET} ${MUTED}From here on:${RESET}\n"
  printf "\n"
  cmd "claude-agents update" "the latest release"
  cmd "claude-agents update --check" "is there a new one?"
  cmd "claude-agents update --to 1.0.0" "that version, downgrades included"
  cmd "claude-agents update --force" "reinstall the one you have"
  printf "\n"
  note "to install with this script anyway:"
  note "curl -fsSL ${INSTALL_URL} | CLAUDE_AGENTS_FORCE=1 sh"
  printf "\n"
  exit 0
fi

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
  kv "version" "${MUTED}v${CURRENT_VERSION}${RESET}  ${LINE}→${RESET}  ${CLAUDE}v${VERSION}${RESET}"
fi
kv "package" "$PACKAGE"
kv "install to" "${INSTALL_DIR}/${BINARY}"

# ── confirmation ─────────────────────────────────────────────────────────────
# Piped into sh, stdin is the script: the answer is read from the terminal.

if [ -t 0 ] || { [ -c /dev/tty ] && (: </dev/tty) 2>/dev/null; }; then
  printf "\n"
  if [ "$MODE" = "reinstall" ]; then
    printf "     ${MUTED}already at v${VERSION} · reinstall?${RESET} ${INK}[y/N]${RESET} "
    read -r _reply </dev/tty
    case "$_reply" in
      [yY]*) ;;
      *) printf "\n     ${MUTED}nothing was touched${RESET}\n\n"; exit 0 ;;
    esac
  else
    _action="install"
    [ "$MODE" = "upgrade" ] && _action="upgrade"
    printf "     ${MUTED}press${RESET} ${INK}enter${RESET} ${MUTED}to ${_action}, or${RESET} ${INK}ctrl+c${RESET} ${MUTED}to cancel${RESET} "
    read -r _ </dev/tty
  fi
fi

# ── step 3: download & verify ────────────────────────────────────────────────

rule 3 download

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

rule 4 install

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

rule "" ready

cmd "claude-agents" "interactive screen (agents used in the last 10 min)"
cmd "claude-agents -m 60" "widen the active window to 60 minutes"
cmd "claude-agents --plain" "print the table and exit"
cmd "claude-agents --plain -v" "the table plus open Claude sessions"
cmd "claude-agents -r ~/code" "search ~/code instead of your home (repeatable)"
cmd "claude-agents update" "when there is a new release"
cmd "claude-agents --help" "every flag"
printf "\n"
note "the screen tells you when a new release is out;"
note "CLAUDE_AGENTS_NO_UPDATE_CHECK=1 turns that check off."
printf "\n"
