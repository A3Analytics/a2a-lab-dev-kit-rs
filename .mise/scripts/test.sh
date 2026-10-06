#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

image="quay.io/keycloak/keycloak:26.8.0@sha256:b0f60d489d51c5d113390bdf5461d4c06e6051be026c05549f2e1e10ec352bcc"
name="a2a-auth-$$"

if ! command -v docker >/dev/null 2>&1; then
  echo "docker is required to run the test suite" >&2
  exit 1
fi

mkdir -p "$root/target"
log="$root/target/a2a-auth-keycloak.log"

cleanup() {
  docker logs "$name" >"$log" 2>&1 || true
  docker rm -f "$name" >/dev/null 2>&1 || true
}
trap cleanup EXIT

docker run -d --name "$name" \
  -p "127.0.0.1::8080" \
  -e KC_BOOTSTRAP_ADMIN_USERNAME=admin \
  -e KC_BOOTSTRAP_ADMIN_PASSWORD=admin \
  -e KC_HEALTH_ENABLED=true \
  -e KC_HOSTNAME_STRICT=false \
  -v "$root/.mise/a2a-auth/realm.json:/opt/keycloak/data/import/a2a-test-realm.json:ro" \
  "$image" \
  start-dev --import-realm --http-port=8080 \
  >/dev/null
port="$(docker port "$name" 8080/tcp)"
port="${port##*:}"
if [ -z "$port" ]; then
  echo "Keycloak did not publish a loopback port" >&2
  exit 1
fi

discovery="http://127.0.0.1:${port}/realms/a2a-test/.well-known/openid-configuration"
ready=0
for _ in $(seq 1 180); do
  if curl -fsS "$discovery" >"$root/target/a2a-auth-discovery.json" 2>/dev/null; then
    ready=1
    break
  fi
  sleep 1
done
if [ "$ready" -ne 1 ]; then
  echo "Keycloak did not become ready: $discovery" >&2
  docker logs "$name" >&2 || true
  exit 1
fi

issuer="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["issuer"])' "$root/target/a2a-auth-discovery.json")"
updated=0
for _ in $(seq 1 30); do
  if docker exec "$name" /opt/keycloak/bin/kcadm.sh config credentials \
    --server http://127.0.0.1:8080 --realm master --user admin --password admin >/dev/null 2>&1 \
    && expired_id="$(docker exec "$name" /opt/keycloak/bin/kcadm.sh get clients -r a2a-test -q clientId=a2a-expired --fields id --format csv --noquotes | tail -n 1)" \
    && docker exec "$name" /opt/keycloak/bin/kcadm.sh update "clients/${expired_id}" -r a2a-test \
      -s 'attributes."access.token.lifespan"=1' >/dev/null 2>&1; then
    updated=1
    break
  fi
  sleep 1
done
if [ "$updated" -ne 1 ]; then
  echo "Keycloak did not accept the expired-client token lifespan" >&2
  exit 1
fi
export A2A_AUTH_ISSUER="$issuer"

if [ "$#" -eq 0 ]; then
  cargo nextest run --workspace --all-features
else
  cargo nextest run --workspace --all-features "$@"
fi
