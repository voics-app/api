#!/usr/bin/env bash
# Runs only with separately checked-out, pinned repositories in a CI job.
set -euo pipefail
: "${QA_API_DIR:?}" "${QA_MESSAGES_DIR:?}" "${QA_GATEWAY_DIR:?}" "${QA_GITOPS_DIR:?}" "${QA_VOICE_DIR:?}"
: "${GITHUB_RUN_ID:?}" "${GITHUB_RUN_ATTEMPT:?}"
: "${GITHUB_WORKSPACE:?}"
project="voics-insights-${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT}"
[[ "$project" =~ ^voics-insights-[0-9]+-[0-9]+$ ]] || exit 2
export QA_IMAGE_PREFIX="qa-insights-${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT}"
export QA_SCRIPT="$(realpath "$QA_API_DIR/qa/insights/insights.mjs")"
local_dir="$(realpath "$QA_GITOPS_DIR/local")"
[[ "$local_dir" == "$(realpath "$GITHUB_WORKSPACE/gitops/local")" ]] || { echo 'Unexpected gitops checkout path' >&2; exit 2; }
[[ ! -e "$local_dir/.env" ]] || { echo 'Refusing a non-disposable gitops checkout' >&2; exit 2; }
(
  cd "$local_dir"
  ./gen-dev-keys.sh
)
export INTERNAL_SERVICE_TOKEN="$(openssl rand -hex 32)"
export MINIO_SECRET_KEY="$(openssl rand -hex 32)"
export VOICE_TOKEN_SECRET="$(openssl rand -hex 32)"
export TURN_STATIC_AUTH_SECRET="$(openssl rand -hex 32)"
export RELEASE_COOKIE="$(openssl rand -hex 32)"
compose=(docker compose --project-name "$project" --project-directory "$local_dir" --env-file "$local_dir/.env" -f "$local_dir/compose.yaml" -f "$QA_API_DIR/qa/insights/compose.override.yaml" --profile full --profile qa)
cleanup() {
  "${compose[@]}" down --volumes --remove-orphans || true
  for service in api messages gateway voice-sfu voice-signaling; do
    docker image rm "$QA_IMAGE_PREFIX/$service:fixture" >/dev/null 2>&1 || true
  done
  rm -f -- "$local_dir/.env"
}
trap cleanup EXIT

docker build -t "$QA_IMAGE_PREFIX/api:fixture" "$QA_API_DIR"
docker build -t "$QA_IMAGE_PREFIX/messages:fixture" "$QA_MESSAGES_DIR"
docker build -t "$QA_IMAGE_PREFIX/gateway:fixture" "$QA_GATEWAY_DIR"
docker build -t "$QA_IMAGE_PREFIX/voice-sfu:fixture" "$QA_VOICE_DIR/sfu"
docker build -t "$QA_IMAGE_PREFIX/voice-signaling:fixture" "$QA_VOICE_DIR/signaling"
"${compose[@]}" up -d api messages gateway voice-signaling
"${compose[@]}" run --rm --no-deps insights-qa
