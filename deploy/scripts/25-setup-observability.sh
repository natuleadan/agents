#!/usr/bin/env bash
set -euo pipefail

# Provision or update telemetry, OpenTelemetry collector wiring, and disk retention caps.
ENV_FILE="${AGENTS_ENV_FILE:-/etc/agents/agents.env}"
LOG_DIR="${AGENTS_LOG_DIR:-/var/log/agents}"
ENABLE_OTEL="${AGENTS_ENABLE_OTEL:-0}"
OTEL_ENDPOINT="${AGENTS_OTEL_ENDPOINT:-http://127.0.0.1:4317}"
RETENTION_DAYS="${AGENTS_RETENTION_DAYS:-30}"
MAX_DISK_PERCENT="${AGENTS_MAX_DISK_PERCENT:-10}"
DRY_RUN="${AGENTS_DRY_RUN:-0}"

usage() {
  cat <<'EOF'
Usage: 25-setup-observability.sh [--enable-otel 0|1] [--otel-endpoint url] [--retention-days N] [--max-disk-percent N] [--dry-run]
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --enable-otel)
      ENABLE_OTEL="${2:?missing value for --enable-otel}"
      shift 2
      ;;
    --otel-endpoint)
      OTEL_ENDPOINT="${2:?missing value for --otel-endpoint}"
      shift 2
      ;;
    --retention-days)
      RETENTION_DAYS="${2:?missing value for --retention-days}"
      shift 2
      ;;
    --max-disk-percent)
      MAX_DISK_PERCENT="${2:?missing value for --max-disk-percent}"
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

echo "[INFO] Configuring Agents Observability & Retention (OTEL=${ENABLE_OTEL}, retention=${RETENTION_DAYS}d, disk_cap=${MAX_DISK_PERCENT}%)"

# Calculate filesystem capacity
ROOT_FS_KB=$(df -k / | awk 'NR==2 {print $2}')
MAX_LOG_KB=$((ROOT_FS_KB * MAX_DISK_PERCENT / 100))
MAX_LOG_MB=$((MAX_LOG_KB / 1024))
echo "[INFO] Filesystem total: ${ROOT_FS_KB} KB. Calculated ${MAX_DISK_PERCENT}% quota: ${MAX_LOG_MB} MB."

if [[ "${DRY_RUN}" != "1" && "$(id -u)" -ne 0 ]]; then
  echo "[FAIL] 25-setup-observability.sh must run as root (use --dry-run to inspect)" >&2
  exit 1
fi

if [[ "${DRY_RUN}" == "1" ]]; then
  echo "[DRY-RUN] ensure ${LOG_DIR} exists with ownership agents:agents"
  echo "[DRY-RUN] update ${ENV_FILE} with AGENTS_OTEL_ENDPOINT"
  echo "[DRY-RUN] install /usr/local/bin/agents-log-prune and cron.daily"
  exit 0
fi

# Ensure log directory exists
mkdir -p "${LOG_DIR}"
if id -u agents >/dev/null 2>&1; then
  chown -R agents:agents "${LOG_DIR}"
fi
chmod 750 "${LOG_DIR}"

# Update environment file if present
if [[ -f "${ENV_FILE}" ]]; then
  sed -i '/^AGENTS_LOG_DIR=/d' "${ENV_FILE}"
  echo "AGENTS_LOG_DIR=${LOG_DIR}" >>"${ENV_FILE}"

  if [[ "${ENABLE_OTEL}" == "1" ]]; then
    sed -i '/^AGENTS_OTEL_ENDPOINT=/d' "${ENV_FILE}"
    sed -i '/^OTLP_ENDPOINT=/d' "${ENV_FILE}"
    echo "AGENTS_OTEL_ENDPOINT=${OTEL_ENDPOINT}" >>"${ENV_FILE}"
    echo "OTLP_ENDPOINT=${OTEL_ENDPOINT}" >>"${ENV_FILE}"
    echo "[INFO] Configured OTLP endpoint: ${OTEL_ENDPOINT}"
  else
    sed -i '/^AGENTS_OTEL_ENDPOINT=/d' "${ENV_FILE}"
    sed -i '/^OTLP_ENDPOINT=/d' "${ENV_FILE}"
    echo "[INFO] OTEL export disabled (local Prometheus /metrics + JSONL active)"
  fi
fi

# Install daily log retention & disk quota pruning script
cat >/usr/local/bin/agents-log-prune <<PRUNE_EOF
#!/usr/bin/env bash
set -euo pipefail
# Prune logs older than ${RETENTION_DAYS} days or exceeding ${MAX_DISK_PERCENT}% disk cap
LOG_DIR="${LOG_DIR}"
MAX_KB="${MAX_LOG_KB}"
RETENTION_DAYS="${RETENTION_DAYS}"

if [[ -d "\${LOG_DIR}" ]]; then
  # 1. Delete files older than retention days
  find "\${LOG_DIR}" -type f -name '*.jsonl' -mtime "+\${RETENTION_DAYS}" -delete

  # 2. Enforce disk cap by deleting oldest files if total usage exceeds quota
  CURRENT_KB=\$(du -sk "\${LOG_DIR}" | awk '{print \$1}')
  while [[ "\${CURRENT_KB}" -gt "\${MAX_KB}" ]]; do
    OLDEST=\$(find "\${LOG_DIR}" -type f -name '*.jsonl' -printf '%T+ %p\n' | sort | head -n 1 | awk '{print \$2}')
    if [[ -n "\${OLDEST}" && -f "\${OLDEST}" ]]; then
      rm -f "\${OLDEST}"
      CURRENT_KB=\$(du -sk "\${LOG_DIR}" | awk '{print \$1}')
    else
      break
    fi
  done
fi
PRUNE_EOF
chmod 755 /usr/local/bin/agents-log-prune

# Register daily cron job
if [[ -d /etc/cron.daily ]]; then
  ln -sf /usr/local/bin/agents-log-prune /etc/cron.daily/agents-log-prune
fi

echo "[PASS] Observability and log retention configured successfully."
