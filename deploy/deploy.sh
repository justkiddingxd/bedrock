#!/usr/bin/env bash
# Builds bedrock-server on a Linux host (inside a throwaway Rust container,
# so the host needs only Docker) and runs it as the systemd service
# `bedrock-server`. Run it again to update; data and settings stay.
#
#   deploy/deploy.sh root@185.225.191.222 [port]
#
# Settings live in /etc/bedrock/server.env on the host, data in /var/lib/bedrock.
set -euo pipefail

HOST=${1:?usage: deploy/deploy.sh user@host [port]}
PORT=${2:-8790}
cd "$(dirname "$0")/.."

echo "==> uploading sources to $HOST"
tar czf - Cargo.toml Cargo.lock crates deploy/bedrock-server.service | ssh "$HOST" '
  mkdir -p /opt/bedrock/src &&
  find /opt/bedrock/src -mindepth 1 -maxdepth 1 ! -name target -exec rm -rf {} + &&
  tar xzf - -C /opt/bedrock/src'

ssh "$HOST" "PORT=$PORT bash -s" <<'REMOTE'
set -euo pipefail
cd /opt/bedrock/src
# The desktop app is not part of the server build.
sed -i '/"app\/src-tauri",/d' Cargo.toml

echo "==> building (Rust container, 3 CPUs)"
docker run --rm --cpus 3 \
  -v /opt/bedrock/src:/src -v bedrock-cargo:/usr/local/cargo/registry -w /src \
  rust:1-bookworm cargo build --release -p bedrock-server

echo "==> installing"
install -d /opt/bedrock/bin
install -m 755 target/release/bedrock-server /opt/bedrock/bin/bedrock-server.new
mv -f /opt/bedrock/bin/bedrock-server.new /opt/bedrock/bin/bedrock-server
id bedrock >/dev/null 2>&1 || useradd --system --home-dir /var/lib/bedrock --shell /usr/sbin/nologin bedrock
install -d -o bedrock -g bedrock -m 750 /var/lib/bedrock
install -d -m 755 /etc/bedrock
if [ ! -f /etc/bedrock/server.env ]; then
  umask 077
  cat > /etc/bedrock/server.env <<ENV
BEDROCK_BIND=0.0.0.0:$PORT
# open | first-only | closed
BEDROCK_REGISTRATION=open
BEDROCK_NAME=Bedrock
# Optional shared bot for everyone without their own (token from @BotFather):
#BEDROCK_TELEGRAM_TOKEN=
ENV
fi
# Settings added later, as comments, for env files from older installs.
grep -q BEDROCK_WEBHOOK_ALLOW_HOSTS /etc/bedrock/server.env || cat >> /etc/bedrock/server.env <<'ENV'
# Hosts reminder webhooks may call although they are local or private (comma-separated), e.g. 127.0.0.1:
#BEDROCK_WEBHOOK_ALLOW_HOSTS=
ENV
install -m 644 deploy/bedrock-server.service /etc/systemd/system/bedrock-server.service
systemctl daemon-reload
systemctl enable bedrock-server >/dev/null 2>&1
systemctl restart bedrock-server

PORT_NOW=$(sed -n 's/^BEDROCK_BIND=.*:\([0-9]*\)$/\1/p' /etc/bedrock/server.env)
if command -v ufw >/dev/null && ufw status | grep -q "Status: active"; then
  ufw allow "$PORT_NOW/tcp" comment bedrock >/dev/null
fi

# Webhook receivers reject signatures more than 5 minutes off.
if command -v timedatectl >/dev/null && [ "$(timedatectl show -p NTPSynchronized --value 2>/dev/null)" != yes ]; then
  echo "!! the clock is not synchronized by NTP: run timedatectl set-ntp true"
fi

sleep 2
systemctl is-active --quiet bedrock-server && echo "==> bedrock-server is running on port $PORT_NOW" || {
  journalctl -u bedrock-server -n 30 --no-pager
  exit 1
}
REMOTE
