#!/bin/sh
# hito-server .deb/.rpm post-install: the user and folders, then the service. A first install
# enables and starts it (it listens on loopback until /etc/hito/hito.toml says otherwise); an
# upgrade restarts it if it was running. Without a running systemd (containers) only the files
# are set up.
set -eu
if ! getent passwd hito >/dev/null 2>&1; then
  nologin=/usr/sbin/nologin
  [ -x "$nologin" ] || nologin=/sbin/nologin
  [ -x "$nologin" ] || nologin=/bin/false
  if command -v useradd >/dev/null 2>&1; then
    useradd --system --user-group --home-dir /var/lib/hito --no-create-home \
      --shell "$nologin" --comment "Hito chat server" hito
  else
    adduser -S -D -H -h /var/lib/hito -s "$nologin" hito
  fi
fi
mkdir -p /var/lib/hito
chown hito:hito /var/lib/hito
chmod 750 /var/lib/hito
if [ -f /etc/hito/hito.toml ]; then
  chown root:hito /etc/hito/hito.toml
  chmod 640 /etc/hito/hito.toml
fi
if [ -d /run/systemd/system ]; then
  systemctl daemon-reload || true
  # deb: "configure" with the old version as $2 on upgrade; rpm: $1 is 1 on install, 2 on upgrade.
  case "${1:-}:${2:-}" in
    configure: | 1:)
      systemctl enable --now hito-server.service || true ;;
    *)
      systemctl try-restart hito-server.service || true ;;
  esac
  # The daily self-update timer, enabled once: a timer someone disabled stays disabled.
  if [ ! -e /var/lib/hito/.update-timer-enabled ]; then
    systemctl enable --now hito-server-update.timer || true
    touch /var/lib/hito/.update-timer-enabled
  fi
fi
exit 0
