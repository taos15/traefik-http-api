#!/usr/bin/env bash
set -euo pipefail
: "${IMAGE:?set IMAGE to the image or digest to test}"
: "${PLATFORM:?set PLATFORM to linux/amd64 or linux/arm64}"
name="traefik-smoke-${RANDOM}"
cleanup() {
  status=$?
  if (( status != 0 )); then docker logs "$name" || true; fi
  docker rm -fv "$name" >/dev/null 2>&1 || true
  exit "$status"
}
trap cleanup EXIT
docker run -d --name "$name" --platform "$PLATFORM" \
  -p 127.0.0.1::4000 \
  -e DOMAIN=smoke.example.test \
  -e DATABASE_URL=sqlite:///app/data/smoke.db \
  -e ADMIN_BOOTSTRAP_PASSWORD="$(openssl rand -hex 24)" \
  "$IMAGE"
port=$(docker port "$name" 4000/tcp)
ready=false
for ((attempt=0; attempt<60; attempt++)); do
  if [[ "$(curl --silent --fail --max-time 2 "http://$port/api/ping" || true)" == pong ]]; then
    ready=true
    break
  fi
  sleep 1
done
[[ "$ready" == true ]]
docker exec "$name" sh -ec '
  test -s /app/data/smoke.db
  test -s /app/web/dist/app.js
  test -s /app/web/dist/app_bg.wasm
  test -d /app/web/dist/snippets
'
