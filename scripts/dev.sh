#!/usr/bin/env bash
# scripts/dev.sh — GhostCacher developer utility script

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(dirname "$SCRIPT_DIR")"
SIDECAR_URL="${GC_SIDECAR_URL:-http://localhost:8080}"
CP_URL="${GC_CP_URL:-http://localhost:7070}"

BOLD="\033[1m"; TEAL="\033[36m"; GREEN="\033[32m"
YELLOW="\033[33m"; RED="\033[31m"; RESET="\033[0m"

info()  { echo -e "${TEAL}${BOLD}[GC]${RESET} $*"; }
ok()    { echo -e "${GREEN}${BOLD}[OK]${RESET} $*"; }
warn()  { echo -e "${YELLOW}${BOLD}[WARN]${RESET} $*"; }
error() { echo -e "${RED}${BOLD}[ERR]${RESET} $*"; exit 1; }

CMD="${1:-help}"

case "$CMD" in

  up)
    info "Starting GhostCacher local stack..."
    cd "$ROOT"

    # ── Stop system Redis if running (it conflicts on port 6379) ──────────
    if sudo systemctl is-active --quiet redis-server 2>/dev/null || \
       sudo systemctl is-active --quiet redis 2>/dev/null; then
      warn "System Redis detected on port 6379 — stopping it..."
      sudo systemctl stop redis-server 2>/dev/null || true
      sudo systemctl stop redis       2>/dev/null || true
    fi

    # ── Start infrastructure first (Redis, Prometheus, Grafana) ──────────
    docker compose up -d --build redis prometheus grafana

    # ── Wait for Redis healthy ────────────────────────────────────────────
    info "Waiting for Redis to be healthy..."
    STATUS="missing"
    for i in $(seq 1 30); do
      STATUS=$(docker inspect gc-redis --format '{{.State.Health.Status}}' 2>/dev/null || echo "missing")
      [ "$STATUS" = "healthy" ] && break
      printf "."
      sleep 1
    done
    echo ""
    [ "$STATUS" != "healthy" ] && error "Redis failed to become healthy (Current status: $STATUS)."

    # ── Get Redis IP directly (bypass DNS entirely) ───────────────────────
    REDIS_IP=$(docker inspect gc-redis \
      --format '{{range $k,$v := .NetworkSettings.Networks}}{{$v.IPAddress}}{{end}}' \
      2>/dev/null | tr -d '[:space:]')

    # Fallback: read from network inspect
    if [ -z "$REDIS_IP" ]; then
      REDIS_IP=$(docker network inspect ghostcacher_ghostcacher-net \
        --format '{{range .Containers}}{{.Name}}|{{.IPv4Address}}{{"\n"}}{{end}}' \
        2>/dev/null | grep gc-redis | cut -d'|' -f2 | cut -d'/' -f1)
    fi

    [ -z "$REDIS_IP" ] && error "Could not determine Redis IP. Is Redis healthy?"
    info "Redis IP: $REDIS_IP"

    # ── Write Redis IP to .env so compose picks it up ─────────────────────
    if grep -q "^GC_REDIS_IP=" "$ROOT/.env" 2>/dev/null; then
      sed -i "s|^GC_REDIS_IP=.*|GC_REDIS_IP=$REDIS_IP|" "$ROOT/.env"
    else
      echo "GC_REDIS_IP=$REDIS_IP" >> "$ROOT/.env"
    fi

    # ── Start sidecar and control-plane ──────────────────────────────────
    docker compose up -d sidecar control-plane

    # ── Wait for them to stabilise ────────────────────────────────────────
    sleep 5

    # ── Patch /etc/hosts inside each container with Redis IP ─────────────
    # Delete this section from dev.sh:
    # ── Patch /etc/hosts inside each container with Redis IP ─────────────
    # ... (the entire for CONTAINER in gc-sidecar gc-control-plane loop) ...

    # ── Final status ──────────────────────────────────────────────────────
    sleep 5
    docker compose ps

    echo ""
    ok "Stack is up."
    echo ""
    echo -e "  ${BOLD}Sidecar proxy:${RESET}      ${SIDECAR_URL}"
    echo -e "  ${BOLD}Control plane:${RESET}      ${CP_URL}/gc/stats"
    echo -e "  ${BOLD}Prometheus:${RESET}         http://localhost:9091"
    echo -e "  ${BOLD}Grafana:${RESET}            http://localhost:3000  (admin/ghostcacher)"
    echo -e "  ${BOLD}RedisInsight:${RESET}       http://localhost:8001"
    echo ""
    echo -e "  ${TEAL}Set your LLM client base URL:${RESET}"
    echo -e "  export ANTHROPIC_BASE_URL=${SIDECAR_URL}"
    ;;

  down)
    info "Stopping GhostCacher local stack..."
    cd "$ROOT"
    docker compose down --remove-orphans --volumes
    sed -i '/^GC_REDIS_IP=/d' "$ROOT/.env" 2>/dev/null || true
    ok "Stack stopped."
    ;;

  logs)
    cd "$ROOT"
    docker compose logs -f --tail=100 sidecar control-plane
    ;;

  build)
    info "Building all GhostCacher Rust binaries (release)..."
    cd "$ROOT"
    cargo build --release --workspace
    ok "Build complete."
    ls -lh target/release/ghostcacher-*
    ;;

  test)
    info "Running test suite..."
    cd "$ROOT"
    cargo test --workspace -- --nocapture
    ok "All tests passed."
    ;;

  bench)
    info "Running cache hit-ratio benchmark..."
    cd "$ROOT"
    cargo bench --package ghostcacher-sidecar
    ;;

  flush)
    SCOPE="${2:-session}"
    info "Flushing cache (scope: $SCOPE)..."
    RESULT=$(curl -s -X POST "${SIDECAR_URL}/gc/flush" \
      -H "Content-Type: application/json" \
      -d "{\"scope\": \"${SCOPE}\"}")
    echo "$RESULT" | python3 -m json.tool 2>/dev/null || echo "$RESULT"
    ok "Flush complete."
    ;;

  status)
    info "Sidecar status:"
    curl -s "${SIDECAR_URL}/gc/status" | python3 -m json.tool 2>/dev/null
    echo ""
    info "Control plane cluster stats:"
    curl -s "${CP_URL}/gc/stats" | python3 -m json.tool 2>/dev/null
    ;;

  redis-cli)
    info "Connecting to GhostCacher Redis..."
    cd "$ROOT"
    docker compose exec redis redis-cli
    ;;

  metrics)
    info "Sidecar Prometheus metrics:"
    curl -s "${SIDECAR_URL%:8080}:9090/metrics"
    ;;

  k8s-apply)
    info "Applying Kubernetes manifests..."
    kubectl apply -f "$ROOT/k8s/00-namespace.yaml"
    kubectl apply -f "$ROOT/k8s/01-redis.yaml"
    kubectl apply -f "$ROOT/k8s/02-sidecar.yaml"
    kubectl apply -f "$ROOT/k8s/03-control-plane.yaml"
    kubectl apply -f "$ROOT/k8s/04-kv-relay.yaml"
    ok "All manifests applied."
    kubectl -n ghostcacher get pods
    ;;

  k8s-delete)
    warn "Deleting all GhostCacher Kubernetes resources..."
    kubectl delete namespace ghostcacher --ignore-not-found=true
    ok "Resources deleted."
    ;;

  help|*)
    echo -e "${BOLD}GhostCacher Dev CLI${RESET}"
    echo ""
    echo "Usage: ./scripts/dev.sh <command> [args]"
    echo ""
    echo "Commands:"
    echo "  up             Start local stack (Docker Compose)"
    echo "  down           Stop local stack"
    echo "  logs           Tail service logs"
    echo "  build          Build all Rust binaries"
    echo "  test           Run test suite"
    echo "  bench          Run benchmarks"
    echo "  flush [scope]  Flush cache (scope: session|docs|system|all)"
    echo "  status         Print sidecar + control plane status"
    echo "  redis-cli      Drop into Redis CLI"
    echo "  metrics        Print Prometheus metrics"
    echo "  k8s-apply      Apply Kubernetes manifests"
    echo "  k8s-delete     Delete Kubernetes resources"
    ;;
esac
