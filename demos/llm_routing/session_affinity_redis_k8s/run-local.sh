#!/usr/bin/env bash
# run-local.sh — Build and run the k8s session affinity demo entirely locally with kind.
# No registry, no image push required.
#
# Usage:
#   ./run-local.sh               # create cluster (if needed), build, deploy, verify
#   ./run-local.sh --build-only  # build and load the image into kind
#   ./run-local.sh --deploy-only # skip build, re-apply k8s manifests
#   ./run-local.sh --verify      # run verify_affinity.py against the running cluster
#   ./run-local.sh --down        # tear down k8s resources (keeps kind cluster)
#   ./run-local.sh --delete-cluster  # also delete the kind cluster

set -euo pipefail

DEMO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$DEMO_DIR/../../.." && pwd)"
IMAGE_NAME="plano-redis:local"
KIND_CLUSTER="plano-demo"

# ---------------------------------------------------------------------------
# Prereq check
# ---------------------------------------------------------------------------

check_prereqs() {
  local missing=()
  command -v docker  >/dev/null 2>&1 || missing+=("docker")
  command -v kubectl >/dev/null 2>&1 || missing+=("kubectl")
  command -v kind    >/dev/null 2>&1 || missing+=("kind  (https://kind.sigs.k8s.io/docs/user/quick-start/#installation)")
  command -v python3 >/dev/null 2>&1 || missing+=("python3")

  if [ ${#missing[@]} -gt 0 ]; then
    echo "ERROR: missing required tools:"
    for t in "${missing[@]}"; do echo "  - $t"; done
    exit 1
  fi
}

load_env() {
  if [ -f "$DEMO_DIR/.env" ]; then
    set -a
    # shellcheck disable=SC1091
    source "$DEMO_DIR/.env"
    set +a
  fi
}

# ---------------------------------------------------------------------------
# Cluster lifecycle
# ---------------------------------------------------------------------------

ensure_cluster() {
  if kind get clusters 2>/dev/null | grep -q "^${KIND_CLUSTER}$"; then
    echo "==> kind cluster '$KIND_CLUSTER' already exists, reusing."
  else
    echo "==> Creating kind cluster '$KIND_CLUSTER'..."
    kind create cluster --name "$KIND_CLUSTER"
    echo "    Cluster created."
  fi

  # Point kubectl at this cluster
  kubectl config use-context "kind-${KIND_CLUSTER}" >/dev/null
}

# ---------------------------------------------------------------------------
# Build and load
# ---------------------------------------------------------------------------

build() {
  echo "==> Building image '$IMAGE_NAME' from repo root..."
  docker build \
    --file "$DEMO_DIR/Dockerfile" \
    --tag "$IMAGE_NAME" \
    --progress=plain \
    "$REPO_ROOT"

  echo "==> Loading '$IMAGE_NAME' into kind cluster '$KIND_CLUSTER'..."
  kind load docker-image "$IMAGE_NAME" --name "$KIND_CLUSTER"
  echo "    Image loaded."
}

# ---------------------------------------------------------------------------
# Deploy / verify / teardown
# ---------------------------------------------------------------------------

deploy() {
  echo ""
  echo "==> Deploying to Kubernetes..."
  "$DEMO_DIR/deploy.sh"
}

verify() {
  echo ""
  echo "==> Running cross-replica verification..."
  python3 "$DEMO_DIR/verify_affinity.py"
}

down() {
  "$DEMO_DIR/deploy.sh" --destroy
}

delete_cluster() {
  echo "==> Deleting kind cluster '$KIND_CLUSTER'..."
  kind delete cluster --name "$KIND_CLUSTER"
  echo "    Cluster deleted."
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

case "${1:-}" in
  --build-only)
    check_prereqs
    load_env
    ensure_cluster
    build
    ;;
  --deploy-only)
    check_prereqs
    load_env
    ensure_cluster
    deploy
    ;;
  --verify)
    check_prereqs
    verify
    ;;
  --down)
    check_prereqs
    down
    ;;
  --delete-cluster)
    check_prereqs
    down
    delete_cluster
    ;;
  "")
    check_prereqs
    load_env
    ensure_cluster
    echo ""
    build
    deploy
    echo ""
    echo "==> Everything is up. Running verification in 5 seconds..."
    echo "    (Ctrl-C to skip — run manually with: ./run-local.sh --verify)"
    sleep 5
    verify
    ;;
  *)
    echo "Usage: $0 [--build-only | --deploy-only | --verify | --down | --delete-cluster]"
    exit 1
    ;;
esac
