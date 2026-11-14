#!/usr/bin/env bash
set -euo pipefail
ENVIRONMENT="$1"
aws s3 sync dist "s3://acme-$ENVIRONMENT"
kubectl rollout restart deployment/web
