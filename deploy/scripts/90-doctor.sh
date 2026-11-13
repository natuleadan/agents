#!/usr/bin/env bash
set -euo pipefail

# Non-destructive health and security audit reporting PASS/FAIL/SKIP.
ROLE="monolithic"
ENV_FILE="${AGENTS_ENV_FILE:-/etc/agents/agents.env}"
SSHD_CONF="${AGENTS_SSHD_CONF:-/etc/ssh/sshd_config.d/99-agents-hardening.conf}"
NON_DESTRUCTIVE="1"

usage() {
  cat <<'EOF'
Usage: 90-doctor.sh [--role monolithic|server|boxer] [--env-file path] [--non-destructive]
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --role)
      ROLE="${2:?missing value for --role}"
      shift 2
      ;;
    --env-file)
      ENV_FILE="${2:?missing value for --env-file}"
      shift 2
      ;;
    --non-destructive)
      NON_DESTRUCTIVE="1"
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
  echo "[FAIL] $1"
  FAILURES=$((FAILURES + 1))
}

skip() {
  echo "[SKIP] $1"
}

check_cmd() {
  local bin="$1"
  local label="$2"
  if command -v "${bin}" >/dev/null 2>&1; then
    pass "Binary available: ${label} (${bin})"
  else
    skip "Binary not installed in current host: ${label} (${bin})"
  fi
}

echo "[INFO] Running non-destructive doctor (role=${ROLE}, non_destructive=${NON_DESTRUCTIVE})"

if [[ -f "${ENV_FILE}" ]]; then
  PERMS="$(stat -c '%a' "${ENV_FILE}")"
  if [[ "${PERMS}" == "600" || "${PERMS}" == "640" || "${PERMS}" == "400" ]]; then
    pass "Environment file permissions safe (${PERMS}): ${ENV_FILE}"
  else
    fail "Environment file permissions too open (${PERMS}); expected 600 or 640"
  fi
  for key in AGENTS_JWT_SECRET AGENTS_CREDENTIAL_MASTER_KEY; do
    if grep -qE "^${key}=.+" "${ENV_FILE}"; then
      pass "Required secret key populated: ${key}"
    else
      fail "Required secret key empty or missing: ${key}"
    fi
  done
else
  fail "Environment file missing: ${ENV_FILE}"
fi

if [[ -f "${SSHD_CONF}" ]]; then
  if grep -q '^PasswordAuthentication no' "${SSHD_CONF}"; then
    pass "SSH password authentication disabled in ${SSHD_CONF}"
  else
    fail "SSH hardening missing 'PasswordAuthentication no' in ${SSHD_CONF}"
  fi
else
  skip "SSH hardening drop-in not present (${SSHD_CONF})"
fi

check_cmd bash "Bash runtime"
check_cmd ssh "OpenSSH client"
if [[ "${ROLE}" == "monolithic" || "${ROLE}" == "server" ]]; then
  check_cmd ufw "UFW firewall"
  check_cmd psql "PostgreSQL client"
fi
if [[ "${ROLE}" == "monolithic" || "${ROLE}" == "boxer" ]]; then
  check_cmd docker "Docker runtime"
fi

if command -v systemctl >/dev/null 2>&1 && systemctl is-system-running >/dev/null 2>&1; then
  if [[ "${ROLE}" == "monolithic" || "${ROLE}" == "server" ]]; then
    if systemctl is-active --quiet agents-server.service; then
      pass "Service active: agents-server.service"
    else
      fail "Service inactive: agents-server.service"
    fi
  fi
  if [[ "${ROLE}" == "monolithic" || "${ROLE}" == "boxer" ]]; then
    if systemctl is-active --quiet agents-boxer.service; then
      pass "Service active: agents-boxer.service"
    else
      fail "Service inactive: agents-boxer.service"
    fi
  fi
else
  skip "systemd PID 1 not running in container/sandbox environment"
fi

if [[ "${FAILURES}" -gt 0 ]]; then
  echo "[FAIL] Doctor completed with ${FAILURES} failure(s)"
  exit 1
fi
pass "Doctor diagnostics passed"
