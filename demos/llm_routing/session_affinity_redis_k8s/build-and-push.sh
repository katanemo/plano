#!/usr/bin/env bash
# build-and-push.sh — Build the Plano demo image and push it to your registry.
#
# Usage:
#   ./build-and-push.sh <registry/image:tag>
#
# Example:
#   ./build-and-push.sh ghcr.io/yourorg/plano-redis:latest
#   ./build-and-push.sh docker.io/youruser/plano-redis:0.4.17
#
# The build context is the repository root. Run this script from anywhere —
# it resolves the repo root automatically.

set -euo pipefail

IMAGE="${1:-}"
if [ -z "$IMAGE" ]; then
  echo "Usage: $0 <registry/image:tag>"
  echo ""
  echo "Example:"
  echo "  $0 ghcr.io/yourorg/plano-redis:latest"
  exit 1
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
DOCKERFILE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/Dockerfile"

echo "Repository root : $REPO_ROOT"
echo "Dockerfile      : $DOCKERFILE"
echo "Image           : $IMAGE"
echo ""

echo "==> Building image (this takes a few minutes — Rust compile from scratch)..."
docker build \
  --file "$DOCKERFILE" \
  --tag "$IMAGE" \
  --progress=plain \
  "$REPO_ROOT"

echo "==> Pushing $IMAGE..."
docker push "$IMAGE"

echo ""
echo "Done. Update k8s/plano.yaml:"
echo "  image: $IMAGE"
echo ""
echo "Then deploy with: ./deploy.sh"
