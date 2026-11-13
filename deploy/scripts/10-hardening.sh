#!/usr/bin/env bash
set -euo pipefail

# Host security hardening: SSH ed25519 key-only, strict UFW, Fail2Ban, CrowdSec, sysctl.
ROLE="monolithic"
VLAN_CIDR="10.0.0.0/8"
SSH_PORT="22"
DRY_RUN="${AGENTS_DRY_RUN:-0}"

usage() {
  cat <<'EOF'
Usage: 10-hardening.sh --role monolithic|server|boxer [--vlan-cidr 10.0.0.0/8] [--ssh-port 22] [--dry-run]
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --role)
      ROLE="${2:?missing value for --role}"
      shift 2
      ;;
    --vlan-cidr)
      VLAN_CIDR="${2:?missing value for --vlan-cidr}"
      shift 2
      ;;
    --ssh-port)
      SSH_PORT="${2:?missing value for --ssh-port}"
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

if [[ -z "${VLAN_CIDR}" ]]; then
  echo "[FAIL] --vlan-cidr must not be empty" >&2
  exit 2
fi

run_cmd() {
  if [[ "${DRY_RUN}" == "1" ]]; then
    echo "[DRY-RUN] $*"
  else
    "$@"
  fi
}

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

if [[ "${DRY_RUN}" != "1" && "$(id -u)" -ne 0 ]]; then
  echo "[FAIL] 10-hardening.sh must be executed as root (use --dry-run to inspect)" >&2
  exit 1
fi

echo "[INFO] Installing hardening packages (role=${ROLE})"
if [[ "${DRY_RUN}" == "1" ]]; then
  echo "[DRY-RUN] apt-get update && apt-get install -y ufw fail2ban unattended-upgrades apt-listchanges"
else
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -y
  apt-get install -y ufw fail2ban unattended-upgrades apt-listchanges
  if apt-cache show crowdsec >/dev/null 2>&1; then
    apt-get install -y crowdsec crowdsec-firewall-bouncer-iptables || true
  fi
fi

write_file /etc/ssh/sshd_config.d/99-agents-hardening.conf <<'EOF'
# Managed by agents-installer
PasswordAuthentication no
KbdInteractiveAuthentication no
ChallengeResponseAuthentication no
PubkeyAuthentication yes
AuthenticationMethods publickey
PermitRootLogin prohibit-password
X11Forwarding no
AllowAgentForwarding no
AllowTcpForwarding no
ClientAliveInterval 300
ClientAliveCountMax 2
EOF

if [[ "${DRY_RUN}" != "1" ]]; then
  sshd -t
  systemctl reload ssh || systemctl reload sshd || true
else
  echo "[DRY-RUN] sshd -t && systemctl reload ssh"
fi

write_file /etc/sysctl.d/99-agents-security.conf <<'EOF'
net.ipv4.tcp_syncookies = 1
net.ipv4.conf.all.rp_filter = 1
net.ipv4.conf.default.rp_filter = 1
net.ipv4.conf.all.accept_redirects = 0
net.ipv4.conf.default.accept_redirects = 0
net.ipv6.conf.all.accept_redirects = 0
fs.protected_hardlinks = 1
fs.protected_symlinks = 1
kernel.kptr_restrict = 2
kernel.dmesg_restrict = 1
EOF
run_cmd sysctl --system

run_cmd ufw --force reset
run_cmd ufw default deny incoming
run_cmd ufw default allow outgoing

case "${ROLE}" in
  monolithic)
    run_cmd ufw allow "${SSH_PORT}/tcp" comment "SSH key-only"
    run_cmd ufw allow 80/tcp comment "Traefik HTTP redirect"
    run_cmd ufw allow 443/tcp comment "Traefik HTTPS/WSS"
    run_cmd ufw allow 443/udp comment "Traefik HTTP/3"
    ;;
  server)
    run_cmd ufw allow "${SSH_PORT}/tcp" comment "SSH key-only"
    run_cmd ufw allow 80/tcp comment "Traefik HTTP redirect"
    run_cmd ufw allow 443/tcp comment "Traefik HTTPS/WSS"
    run_cmd ufw allow 443/udp comment "Traefik HTTP/3"
    run_cmd ufw allow from "${VLAN_CIDR}" to any port 4222 proto tcp comment "NATS VLAN"
    run_cmd ufw allow from "${VLAN_CIDR}" to any port 6379 proto tcp comment "Valkey VLAN"
    ;;
  boxer)
    # Distributed Boxer nodes never expose public ingress; VLAN-only access.
    run_cmd ufw allow from "${VLAN_CIDR}" to any port "${SSH_PORT}" proto tcp comment "SSH VLAN-only"
    run_cmd ufw allow from "${VLAN_CIDR}" to any port 30000:39999 proto tcp comment "Traefik VLAN previews"
    ;;
esac

run_cmd ufw --force enable

write_file /etc/fail2ban/jail.d/99-agents.local <<EOF
[sshd]
enabled = true
port = ${SSH_PORT}
maxretry = 4
bantime = 1h
findtime = 10m
EOF

run_cmd systemctl enable --now fail2ban
run_cmd dpkg-reconfigure -f noninteractive unattended-upgrades

echo "[PASS] Host hardening configured for role=${ROLE}"
