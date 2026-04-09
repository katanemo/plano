#!/usr/bin/env bash
# deploy.sh — Apply all Kubernetes manifests in the correct order.
#
# Usage:
#   ./deploy.sh            # deploy everything
#   ./deploy.sh --destroy  # tear down everything (keeps namespace for safety)
#   ./deploy.sh --status   # show pod and service status

set -euo pipefail

DEMO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
K8S_DIR="$DEMO_DIR/k8s"
NS="plano-demo"

check_prereqs() {
  local missing=()
  command -v kubectl >/dev/null 2>&1 || missing+=("kubectl")
  if [ ${#missing[@]} -gt 0 ]; then
    echo "ERROR: missing required tools: ${missing[*]}"
    exit 1
  fi

  if ! kubectl cluster-info &>/dev/null; then
    echo "ERROR: no Kubernetes cluster reachable. Start minikube/kind or configure kubeconfig."
    exit 1
  fi
}

create_secret() {
  if kubectl get secret plano-secrets -n "$NS" &>/dev/null; then
    echo "    Secret plano-secrets already exists, skipping."
    return
  fi

  local openai_api_key="${OPENAI_API_KEY:-}"
  if [ -z "$openai_api_key" ]; then
    echo ""
    echo "No 'plano-secrets' secret found in namespace '$NS'."
    echo "Enter API keys (input is hidden):"
    echo ""
    read -r -s -p "  OPENAI_API_KEY: " openai_api_key
    echo ""
  else
    echo "    Using OPENAI_API_KEY from environment."
  fi

  if [ -z "$openai_api_key" ]; then
    echo "ERROR: OPENAI_API_KEY cannot be empty."
    exit 1
  fi

  kubectl create secret generic plano-secrets \
    --from-literal=OPENAI_API_KEY="$openai_api_key" \
    -n "$NS"

  echo "    Secret created."
}

deploy() {
  echo "==> Applying namespace..."
  kubectl apply -f "$K8S_DIR/namespace.yaml"

  echo "==> Creating API key secret..."
  create_secret

  echo "==> Applying Redis (StatefulSet + Services)..."
  kubectl apply -f "$K8S_DIR/redis.yaml"

  echo "==> Applying Jaeger..."
  kubectl apply -f "$K8S_DIR/jaeger.yaml"

  echo "==> Applying Plano config (ConfigMap)..."
  kubectl apply -f "$K8S_DIR/plano-config.yaml"

  echo "==> Applying Plano deployment + HPA..."
  kubectl apply -f "$K8S_DIR/plano.yaml"

  echo ""
  echo "==> Waiting for Redis to be ready..."
  kubectl rollout status statefulset/redis -n "$NS" --timeout=120s

  echo "==> Waiting for Plano pods to be ready..."
  kubectl rollout status deployment/plano -n "$NS" --timeout=120s

  echo ""
  echo "Deployment complete!"
  show_status
  echo ""
  echo "Useful commands:"
  echo "  # Tail logs from all Plano pods:"
  echo "  kubectl logs -l app=plano -n $NS -f"
  echo ""
  echo "  # Open Jaeger UI:"
  echo "  kubectl port-forward svc/jaeger 16686:16686 -n $NS &"
  echo "  open http://localhost:16686"
  echo ""
  echo "  # Access Redis CLI:"
  echo "  kubectl exec -it redis-0 -n $NS -- redis-cli"
  echo ""
  echo "  # Run the verification script:"
  echo "  python $DEMO_DIR/verify_affinity.py"
}

destroy() {
  echo "==> Deleting Plano, Jaeger, and Redis resources..."
  kubectl delete -f "$K8S_DIR/plano.yaml"     --ignore-not-found
  kubectl delete -f "$K8S_DIR/jaeger.yaml"    --ignore-not-found
  kubectl delete -f "$K8S_DIR/redis.yaml"     --ignore-not-found
  kubectl delete -f "$K8S_DIR/plano-config.yaml" --ignore-not-found
  kubectl delete secret plano-secrets -n "$NS" --ignore-not-found

  echo ""
  echo "Resources deleted."
  echo "Namespace '$NS' was kept. Remove it manually if desired:"
  echo "  kubectl delete namespace $NS"
}

show_status() {
  echo ""
  echo "=== Pods ==="
  kubectl get pods -n "$NS" -o wide
  echo ""
  echo "=== Services ==="
  kubectl get svc -n "$NS"
  echo ""
  echo "=== HPA ==="
  kubectl get hpa -n "$NS" 2>/dev/null || true
}

check_prereqs

case "${1:-}" in
  --destroy)  destroy ;;
  --status)   show_status ;;
  *)          deploy ;;
esac
