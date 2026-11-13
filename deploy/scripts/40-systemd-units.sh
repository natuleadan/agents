#!/usr/bin/env bash
set -euo pipefail

# Install hardened systemd service units and logrotate rules for agents-server and agents-boxer.
ROLE="monolithic"
ENV_FILE="/etc/agents/agents.env"
BIN_DIR="/usr/local/bin"
DRY_RUN="${AGENTS_DRY_RUN:-0}"

usage() {
  cat <<'EOF'
Usage: 40-systemd-units.sh [--role monolithic|server|boxer] [--env-file path] [--bin-dir dir] [--dry-run]
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
    --bin-dir)
      BIN_DIR="${2:?missing value for --bin-dir}"
      shift 2
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

if [[ "${DRY_RUN}" != "1" && "$(id -u)" -ne 0 ]]; then
  echo "[FAIL] 40-systemd-units.sh must run as root (use --dry-run to inspect)" >&2
  exit 1
fi

write_file() {
  local dest="$1"
  if [[ "${DRY_RUN}" == "1" ]]; then
    echo "[DRY-RUN] write ${dest}"
    cat >/dev/null
  else
    mkdir -p "$(dirname "${dest}")"
    cat >"${dest}"
  fi
}

run_cmd() {
  if [[ "${DRY_RUN}" == "1" ]]; then
    echo "[DRY-RUN] $*"
  else
    "$@"
  fi
}

if [[ "${DRY_RUN}" != "1" ]]; then
  if ! id -u agents >/dev/null 2>&1; then
    useradd --system --no-create-home --shell /usr/sbin/nologin agents
  fi
  install -d -m 0750 -o root -g agents /etc/agents
  install -d -m 0750 -o agents -g agents /var/lib/agents /var/lib/agents/boxes /var/log/agents
else
  echo "[DRY-RUN] ensure unprivileged system user 'agents' and /etc/agents, /var/lib/agents"
fi

if [[ "${ROLE}" == "monolithic" || "${ROLE}" == "server" ]]; then
  write_file /etc/systemd/system/agents-server.service <<EOF
[Unit]
Description=Agents Ironic Orchestrator Server
After=network-online.target postgresql.service
Wants=network-online.target

[Service]
Type=simple
User=agents
Group=agents
EnvironmentFile=${ENV_FILE}
ExecStart=${BIN_DIR}/agents-server
Restart=on-failure
RestartSec=3
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
ProtectKernelTunables=true
ProtectControlGroups=true
CapabilityBoundingSet=
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX
ReadWritePaths=/var/lib/agents /var/log/agents

[Install]
WantedBy=multi-user.target
EOF
fi

if [[ "${ROLE}" == "monolithic" || "${ROLE}" == "boxer" ]]; then
  write_file /etc/systemd/system/agents-boxer.service <<EOF
[Unit]
Description=Agents Boxer Sandbox Supervisor
After=network-online.target docker.service
Wants=network-online.target

[Service]
Type=simple
User=agents
Group=agents
EnvironmentFile=${ENV_FILE}
ExecStart=${BIN_DIR}/agents-boxer
Restart=on-failure
RestartSec=3
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
ProtectKernelTunables=true
ProtectControlGroups=true
CapabilityBoundingSet=
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX
ReadWritePaths=/var/lib/agents /var/log/agents

[Install]
WantedBy=multi-user.target
EOF
fi

write_file /etc/logrotate.d/agents <<'EOF'
/var/log/agents/*.log {
  daily
  rotate 14
  compress
  delaycompress
  missingok
  notifempty
  create 0640 agents agents
}
EOF

run_cmd systemctl daemon-reload
if [[ "${ROLE}" == "monolithic" || "${ROLE}" == "server" ]]; then
  run_cmd systemctl enable --now agents-server.service
fi
if [[ "${ROLE}" == "monolithic" || "${ROLE}" == "boxer" ]]; then
  run_cmd systemctl enable --now agents-boxer.service
fi

echo "[PASS] Hardened systemd units and logrotate installed for role=${ROLE}"
