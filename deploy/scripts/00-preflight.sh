#!/usr/bin/env bash
set -euo pipefail

# Preflight verification for Debian/Ubuntu LTS hosts before provisioning.
OS_RELEASE_FILE="${OS_RELEASE_FILE:-/etc/os-release}"
MIN_RAM_MB="${AGENTS_MIN_RAM_MB:-1024}"
MIN_DISK_MB="${AGENTS_MIN_DISK_MB:-8192}"
ALLOW_NON_ROOT="${AGENTS_ALLOW_NON_ROOT:-0}"
DRY_RUN="${AGENTS_DRY_RUN:-0}"
ROLE="monolithic"

usage() {
  cat <<'EOF'
Usage: 00-preflight.sh [--role monolithic|server|boxer] [--allow-non-root] [--dry-run]
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --role)
      ROLE="${2:?missing value for --role}"
      shift 2
      ;;
    --allow-non-root)
      ALLOW_NON_ROOT="1"
      shift
      ;;
    --dry-run)
      DRY_RUN="1"
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      echo "[FAIL] unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

case "${ROLE}" in
  monolithic | server | boxer) ;;
  *)
    echo "[FAIL] unsupported role: ${ROLE}" >&2
    exit 2
    ;;
esac

FAILURES=0

pass() {
  echo "[PASS] $1"
}

fail() {
  echo "[FAIL] $1" >&2
  FAILURES=$((FAILURES + 1))
}

skip() {
  echo "[SKIP] $1"
}

if [[ ! -r "${OS_RELEASE_FILE}" ]]; then
  fail "Cannot read OS metadata at ${OS_RELEASE_FILE}"
else
  # shellcheck disable=SC1090
  source "${OS_RELEASE_FILE}"
  OS_ID="${ID:-unknown}"
  VERSION_MAJOR="${VERSION_ID%%.*}"
  case "${OS_ID}" in
    ubuntu)
      if [[ "${VERSION_MAJOR:-0}" -ge 22 ]]; then
        pass "Supported OS: ubuntu ${VERSION_ID:-unknown} (role=${ROLE})"
      else
        fail "Ubuntu 22.04+ LTS required, found ${VERSION_ID:-unknown}"
      fi
      ;;
    debian)
      if [[ "${VERSION_MAJOR:-0}" -ge 12 ]]; then
        pass "Supported OS: debian ${VERSION_ID:-unknown} (role=${ROLE})"
      else
        fail "Debian 12+ required, found ${VERSION_ID:-unknown}"
      fi
      ;;
    *)
      fail "Unsupported OS '${OS_ID}'; Debian 12+ or Ubuntu 22.04+ required"
      ;;
  esac
fi

HOST_ARCH="$(uname -m)"
case "${HOST_ARCH}" in
  x86_64 | aarch64 | arm64)
    pass "Supported architecture: ${HOST_ARCH}"
    ;;
  *)
    fail "Unsupported CPU architecture: ${HOST_ARCH}"
    ;;
esac

if [[ -r /proc/meminfo ]]; then
  MEM_KB="$(awk '/MemTotal:/ { print $2; exit }' /proc/meminfo)"
  MEM_MB=$((MEM_KB / 1024))
  if [[ "${MEM_MB}" -ge "${MIN_RAM_MB}" ]]; then
    pass "Memory check: ${MEM_MB} MB >= ${MIN_RAM_MB} MB"
  else
    fail "Insufficient memory: ${MEM_MB} MB < ${MIN_RAM_MB} MB"
  fi
else
  skip "Unable to read /proc/meminfo"
fi

AVAIL_MB="$(df -Pm / | awk 'END { print $(NF-2) }')"
if [[ "${AVAIL_MB:-0}" -ge "${MIN_DISK_MB}" ]]; then
  pass "Disk headroom on /: ${AVAIL_MB} MB >= ${MIN_DISK_MB} MB"
else
  fail "Insufficient disk headroom on /: ${AVAIL_MB:-0} MB < ${MIN_DISK_MB} MB"
fi

if [[ "$(id -u)" -eq 0 ]]; then
  pass "Running with root privileges"
elif [[ "${ALLOW_NON_ROOT}" == "1" || "${DRY_RUN}" == "1" ]]; then
  skip "Non-root execution permitted via flag; real provisioning requires sudo/root"
else
  fail "Provisioning requires root privileges; re-run with sudo or --allow-non-root for inspection"
fi

if [[ "${FAILURES}" -gt 0 ]]; then
  exit 1
fi
pass "Preflight verification complete"
