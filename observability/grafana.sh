#!/usr/bin/env bash
# Starts the local HeraldQ Grafana against the local Postgres and Prometheus.
# Cold-boot usage:  bash observability/grafana.sh
# Then open:        http://localhost:3001/d/heraldq-overview   (anonymous admin)
#
# The dashboard reads from BOTH datasources, so for a full picture you also want
# `bash observability/prometheus.sh` running — without it, the API / queue /
# delivery-rate panels are empty while the Postgres forensic panels still work.
#
# Prereqs:
#   - Postgres running locally (WSL: `sudo service postgresql start`)
#   - Grafana OSS unpacked at $GRAFANA_HOME (default ~/grafana). To (re)install:
#       curl -sL https://dl.grafana.com/oss/release/grafana-13.0.2.linux-amd64.tar.gz \
#         | tar -xz -C ~/grafana --strip-components=1   # mkdir -p ~/grafana first
set -euo pipefail

# Repo root = parent of this script's dir, so provisioning paths resolve anywhere.
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GRAFANA_HOME="${GRAFANA_HOME:-$HOME/grafana}"

# Postgres connection (override via env for a different local DB).
export POSTGRES_HOST="${POSTGRES_HOST:-localhost}"
export POSTGRES_PORT="${POSTGRES_PORT:-5432}"
export POSTGRES_DB="${POSTGRES_DB:-sliq}"
export POSTGRES_USER="${POSTGRES_USER:-postgres}"
export POSTGRES_PASSWORD="${POSTGRES_PASSWORD:-xi23pe}"
export POSTGRES_SSLMODE="${POSTGRES_SSLMODE:-disable}"

# Prometheus datasource (see observability/prometheus.sh).
export PROMETHEUS_URL="${PROMETHEUS_URL:-http://localhost:9090}"

# Grafana config (3001 to avoid clashing with the api's default :3000).
export GF_SERVER_HTTP_PORT="${GF_SERVER_HTTP_PORT:-3001}"
export GF_PATHS_PROVISIONING="$REPO_ROOT/observability/grafana/provisioning"
export GF_PATHS_DATA="$GRAFANA_HOME/data"
export GF_PATHS_LOGS="$GRAFANA_HOME/logs"
export GF_AUTH_ANONYMOUS_ENABLED=true
export GF_AUTH_ANONYMOUS_ORG_ROLE=Admin
export GF_ANALYTICS_REPORTING_ENABLED=false
export GF_ANALYTICS_CHECK_FOR_UPDATES=false

# Consumed by provisioning/dashboards/provider.yml, so the dashboard path is not
# hardcoded to one developer's home directory.
export HERALDQ_DASHBOARDS_PATH="$REPO_ROOT/observability/grafana/dashboards"

if [[ ! -x "$GRAFANA_HOME/bin/grafana" ]]; then
  echo "Grafana binary not found at $GRAFANA_HOME/bin/grafana — see install note at top of this script." >&2
  exit 1
fi

echo "Grafana → http://localhost:$GF_SERVER_HTTP_PORT/d/heraldq-overview  (Ctrl-C to stop)"
cd "$GRAFANA_HOME"
exec ./bin/grafana server --homepath "$GRAFANA_HOME"
