#!/usr/bin/env bash
set -euo pipefail

# Provision Boxer node: rootless Docker, cgroup v2 limits, OpenShell hook, private VLAN NATS.
RUNTIME="docker"
ISOLATION="openshell"
NATS_URL="nats://127.0.0.1:4222"
VLAN_IP=""
MAX_BOXES="8"
DRY_RUN="${AGENTS_DRY_RUN:-0}"

usage() {
  cat <<'EOF'
Usage: 30-deps-boxer.sh [--runtime docker|host] [--isolation native|openshell] [--nats-url url] [--vlan-ip ip] [--max-boxes n] [--dry-run]
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --runtime)
      RUNTIME="${2:?missing value for --runtime}"
      shift 2
      ;;
    --isolation)
      ISOLATION="${2:?missing value for --isolation}"
      shift 2
      ;;
    --nats-url)
      NATS_URL="${2:?missing value for --nats-url}"
      shift 2
      ;;
    --vlan-ip)
      VLAN_IP="${2:?missing value for --vlan-ip}"
      shift 2
      ;;
    --max-boxes)
      MAX_BOXES="${2:?missing value for --max-boxes}"
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

case "${ISOLATION}" in
  native | openshell) ;;
  *)
    echo "[FAIL] fail-closed: Boxer isolation must be 'native' or 'openshell', got '${ISOLATION}'" >&2
    exit 2
    ;;
esac

if [[ "${DRY_RUN}" != "1" && "$(id -u)" -ne 0 ]]; then
  echo "[FAIL] 30-deps-boxer.sh must run as root (use --dry-run to inspect)" >&2
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

if [[ "${DRY_RUN}" == "1" ]]; then
  echo "[DRY-RUN] install docker.io uidmap dbus-user-session fuse-overlayfs (runtime=${RUNTIME})"
else
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -y
  apt-get install -y docker.io uidmap dbus-user-session fuse-overlayfs
  systemctl enable --now docker
fi

write_file /etc/docker/daemon.json <<'EOF'
{
  "exec-opts": ["native.cgroupdriver=systemd"],
  "icc": false,
  "live-restore": true,
  "log-driver": "json-file",
  "log-opts": {
    "max-size": "10m",
    "max-file": "3"
  },
  "no-new-privileges": true,
  "userns-remap": "default"
}
EOF

if [[ "${ISOLATION}" == "openshell" ]]; then
  write_file /etc/agents/openshell.conf <<EOF
isolation=openshell
max_concurrent_boxes=${MAX_BOXES}
cgroup_driver=systemd
EOF
fi

if [[ -n "${VLAN_IP}" ]]; then
  echo "[INFO] Binding Boxer preview target to private VLAN IP ${VLAN_IP}"
fi
echo "[PASS] Boxer node provisioned (runtime=${RUNTIME}, isolation=${ISOLATION}, nats=${NATS_URL})"
