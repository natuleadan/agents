#!/usr/bin/env bash
set -euo pipefail

# Non-destructive audit for Prometheus metrics, W3C tracing, health probes, and log retention.
SERVER_URL="${AGENTS_SERVER_URL:-http://127.0.0.1:3000}"
BOXER_URL="${AGENTS_BOXER_URL:-http://127.0.0.1:9091}"
LOG_DIR="${AGENTS_LOG_DIR:-/var/log/agents}"
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

echo "[INFO] Running Agents Observability & Metrics Audit"

# 1. Audit Server /metrics (Prometheus JSON payload or text)
if curl -sf --max-time 3 "${SERVER_URL}/metrics" >/dev/null 2>&1; then
  RESP=$(curl -s --max-time 3 "${SERVER_URL}/metrics")
  if echo "${RESP}" | grep -q "ironic_http_requests_total\|agents_domain_requests_total"; then
    pass "agents-server GET /metrics returns expected Prometheus metrics"
  else
    fail "agents-server GET /metrics did not contain expected counter names"
  fi
else
  skip "agents-server not reachable at ${SERVER_URL} (daemon stopped or different port)"
fi

# 2. Audit Server /metrics/prometheus (raw text format)
if curl -sf --max-time 3 "${SERVER_URL}/metrics/prometheus" >/dev/null 2>&1; then
  CT=$(curl -s -I --max-time 3 "${SERVER_URL}/metrics/prometheus" | grep -i '^content-type:' || true)
  if echo "${CT}" | grep -q "text/plain"; then
    pass "agents-server GET /metrics/prometheus content-type is text/plain"
  else
    fail "agents-server GET /metrics/prometheus invalid content-type: ${CT}"
  fi
else
  skip "agents-server /metrics/prometheus not reachable at ${SERVER_URL}"
fi

# 3. Audit Boxer /metrics
if curl -sf --max-time 3 "${BOXER_URL}/metrics" >/dev/null 2>&1; then
  BOX_RESP=$(curl -s --max-time 3 "${BOXER_URL}/metrics")
  if echo "${BOX_RESP}" | grep -q "boxer_active_boxes\|boxer_provisions_total"; then
    pass "agents-boxer GET /metrics returns expected supervisor metrics"
  else
    fail "agents-boxer GET /metrics missing boxer_* metrics"
  fi
else
  skip "agents-boxer not reachable at ${BOXER_URL}"
fi

# 4. Audit Server /health providers indicator
if curl -sf --max-time 3 "${SERVER_URL}/health" >/dev/null 2>&1; then
  HEALTH_JSON=$(curl -s --max-time 3 "${SERVER_URL}/health")
  if echo "${HEALTH_JSON}" | grep -q '"providers"'; then
    pass "agents-server /health reports Container::health DI 'providers' indicator"
  else
    fail "agents-server /health missing 'providers' check entry"
  fi
else
  skip "agents-server /health not reachable at ${SERVER_URL}"
fi

# 5. Audit W3C traceparent and x-trace-id header propagation
if curl -sf --max-time 3 -I "${SERVER_URL}/health/live" >/dev/null 2>&1; then
  HEADERS=$(curl -s -I --max-time 3 "${SERVER_URL}/health/live")
  if echo "${HEADERS}" | grep -qi "traceparent:"; then
    pass "agents-server injects W3C traceparent response header"
  else
    fail "agents-server missing W3C traceparent header in response"
  fi
  if echo "${HEADERS}" | grep -qi "x-trace-id:"; then
    pass "agents-server injects x-trace-id correlation response header"
  else
    fail "agents-server missing x-trace-id header in response"
  fi
else
  skip "agents-server /health/live not reachable for header audit"
fi

# 6. Audit gzip compression
if curl -sf --max-time 3 -H "Accept-Encoding: gzip" -I "${SERVER_URL}/openapi.json" >/dev/null 2>&1; then
  ENC=$(curl -s -H "Accept-Encoding: gzip" -I --max-time 3 "${SERVER_URL}/openapi.json" | grep -i '^content-encoding:' || true)
  if echo "${ENC}" | grep -qi "gzip"; then
    pass "agents-server returns Content-Encoding: gzip on Accept-Encoding: gzip"
  else
    fail "agents-server response not compressed with gzip"
  fi
else
  skip "agents-server /openapi.json not reachable for compression audit"
fi

# 7. Audit log directory and retention bounds
if [[ -d "${LOG_DIR}" ]]; then
  pass "Log directory exists: ${LOG_DIR}"
  ROOT_KB=$(df -k / | awk 'NR==2 {print $2}')
  MAX_ALLOWED_KB=$(( ROOT_KB / 10 ))
  CURRENT_KB=$(du -sk "${LOG_DIR}" 2>/dev/null | awk '{print $1}')
  if [[ "${CURRENT_KB}" -le "${MAX_ALLOWED_KB}" ]]; then
    pass "Log usage (${CURRENT_KB} KB) within 10% disk capacity cap (${MAX_ALLOWED_KB} KB)"
  else
    fail "Log usage (${CURRENT_KB} KB) exceeds 10% disk capacity cap (${MAX_ALLOWED_KB} KB)"
  fi
else
  skip "Log directory ${LOG_DIR} not present on this host (local test environment)"
fi

echo "[INFO] Observability audit completed with ${FAILURES} failure(s)."
exit "${FAILURES}"
