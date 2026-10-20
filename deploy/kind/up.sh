#!/usr/bin/env bash
# The whole platform on a local Kubernetes cluster (kind), as in production:
# cert-manager (internal TLS), CloudNativePG (PostgreSQL with TLS), NATS
# JetStream, S3 (RustFS), restricted Pod Security and NetworkPolicies.
#
#   docker build -f deploy/docker/server.Dockerfile -t hebra-server:dev .
#   docker build -f deploy/docker/web.Dockerfile -t hebra-web:dev .
#   deploy/kind/up.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# Git Bash on Windows: native paths for kubectl and helm.
if command -v cygpath >/dev/null; then
  ROOT=$(cygpath -m "$ROOT")
  export MSYS_NO_PATHCONV=1
fi
CLUSTER="${CLUSTER:-hebra}"

kind get clusters | grep -qx "$CLUSTER" || kind create cluster --name "$CLUSTER" --wait 120s

kubectl apply --server-side -f https://github.com/cert-manager/cert-manager/releases/download/v1.21.2/cert-manager.yaml
kubectl apply --server-side -f https://raw.githubusercontent.com/cloudnative-pg/cloudnative-pg/release-1.30/releases/cnpg-1.30.1.yaml
kubectl apply --server-side -f https://github.com/kubernetes-sigs/gateway-api/releases/download/v1.6.3/standard-install.yaml
kubectl -n cert-manager rollout status deploy/cert-manager-webhook --timeout=300s
kubectl -n cnpg-system rollout status deploy/cnpg-controller-manager --timeout=300s

for image in hebra-server:dev hebra-web:dev; do kind load docker-image "$image" --name "$CLUSTER"; done

# Secrets (in production: External Secrets from your KMS / Vault).
rand() { head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n'; }
kubectl create namespace hebra --dry-run=client -o yaml | kubectl apply -f -
kubectl create namespace storage --dry-run=client -o yaml | kubectl apply -f -
S3_KEY="hebra$(rand | head -c 8)"
S3_SECRET="$(rand)"
for ns in hebra storage; do
  name=$([ "$ns" = hebra ] && echo hebra-s3 || echo s3)
  kubectl -n "$ns" get secret "$name" >/dev/null 2>&1 ||
    kubectl -n "$ns" create secret generic "$name" \
      --from-literal=AWS_ACCESS_KEY_ID="$S3_KEY" --from-literal=AWS_SECRET_ACCESS_KEY="$S3_SECRET"
done
kubectl -n hebra get secret hebra-encryption >/dev/null 2>&1 ||
  kubectl -n hebra create secret generic hebra-encryption --from-literal=keys="$(rand)"
kubectl -n hebra get secret hebra-mandate >/dev/null 2>&1 ||
  kubectl -n hebra create secret generic hebra-mandate \
    --from-literal=key="$(docker run --rm hebra-server:dev keygen mandate)"

kubectl apply -f "$ROOT/deploy/kind/prereqs.yaml"
kubectl -n storage rollout status deploy/rustfs --timeout=300s
kubectl -n storage run make-bucket --rm -i --restart=Never --image=amazon/aws-cli:2.38.1 \
  --env AWS_ACCESS_KEY_ID="$(kubectl -n storage get secret s3 -o jsonpath='{.data.AWS_ACCESS_KEY_ID}' | base64 -d)" \
  --env AWS_SECRET_ACCESS_KEY="$(kubectl -n storage get secret s3 -o jsonpath='{.data.AWS_SECRET_ACCESS_KEY}' | base64 -d)" \
  --env AWS_DEFAULT_REGION=us-east-1 \
  -- --endpoint-url http://rustfs.storage:9000 s3 mb s3://hebra || true

helm repo add nats https://nats-io.github.io/k8s/helm/charts/ --force-update >/dev/null
helm dependency build "$ROOT/deploy/helm/hebra"
helm upgrade --install hebra "$ROOT/deploy/helm/hebra" -n hebra \
  -f "$ROOT/deploy/kind/values.yaml" --wait --timeout 10m

kubectl -n hebra get pods
