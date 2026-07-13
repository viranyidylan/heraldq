#!/usr/bin/env bash
# Starts the local HeraldQ Prometheus, scraping the api and worker.
# Cold-boot usage:  bash observability/prometheus.sh
# Then open:        http://localhost:9090/targets   (both jobs should be UP)
#                   http://localhost:9090/alerts    (the rules in alerts/)
#
# Prereqs:
#   - api and worker running with METRICS_PORT set (9464 / 9465 respectively);
#     a target that is DOWN here means that process is stopped or has no
#     METRICS_PORT, in which case its metrics are silent no-ops.
#   - Prometheus unpacked at $PROM_HOME (default ~/prometheus). To (re)install:
#       mkdir -p ~/prometheus && curl -sL \
#         https://github.com/prometheus/prometheus/releases/download/v3.6.0/prometheus-3.6.0.linux-amd64.tar.gz \
#         | tar -xz -C ~/prometheus --strip-components=1
set -euo pipefail

# Repo root = parent of this script's dir, so config paths resolve anywhere.
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROM_HOME="${PROM_HOME:-$HOME/prometheus}"

# 9090 is Prometheus convention; avoids api :3000, Grafana :3001, metrics :946x.
PROM_PORT="${PROM_PORT:-9090}"

if [[ ! -x "$PROM_HOME/prometheus" ]]; then
  echo "Prometheus binary not found at $PROM_HOME/prometheus — see install note at top of this script." >&2
  exit 1
fi

echo "Prometheus → http://localhost:$PROM_PORT/targets  (Ctrl-C to stop)"
exec "$PROM_HOME/prometheus" \
  --config.file="$REPO_ROOT/observability/prometheus/prometheus.yml" \
  --storage.tsdb.path="$PROM_HOME/data" \
  --web.listen-address="0.0.0.0:$PROM_PORT"
