#!/bin/sh
# Install the Hito server from this folder on any Linux with systemd (Debian, Ubuntu, a Proxmox
# LXC, Fedora...). Run as root from the unpacked folder:
#
#   sudo ./install.sh              install or upgrade, then start the service
#   sudo ./install.sh --uninstall  remove the program and the service (keeps the data and config)
#
# It installs /usr/local/bin/hito-server, the configuration /etc/hito/hito.toml (kept if it
# already exists), the data folder /var/lib/hito owned by the `hito` system user, and the
# systemd service hito-server.
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
bin=/usr/local/bin/hito-server
unit=/etc/systemd/system/hito-server.service

if [ "$(id -u)" -ne 0 ]; then
  echo "Run it as root: sudo $0 $*" >&2
  exit 1
fi
systemd=0
[ -d /run/systemd/system ] && systemd=1

if [ "${1:-}" = "--uninstall" ]; then
  if [ "$systemd" = 1 ]; then systemctl disable --now hito-server.service 2>/dev/null || true; fi
  rm -f "$bin" "$unit"
  if [ "$systemd" = 1 ]; then systemctl daemon-reload; fi
  echo "Removed the Hito server. Data in /var/lib/hito and /etc/hito/hito.toml were kept."
  exit 0
fi
if [ -n "${1:-}" ]; then
  sed -n '2,9p' "$0" >&2
  exit 2
fi

"$here/hito-server" --version >/dev/null
install -d -m 755 /usr/local/bin
install -m 755 "$here/hito-server" "$bin.new"
mv -f "$bin.new" "$bin"

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
install -d -o hito -g hito -m 750 /var/lib/hito
install -d -m 755 /etc/hito
if [ ! -e /etc/hito/hito.toml ]; then
  install -o root -g hito -m 640 "$here/hito.toml" /etc/hito/hito.toml
  echo "Wrote /etc/hito/hito.toml"
else
  echo "Kept your /etc/hito/hito.toml"
fi

install -d -m 755 "$(dirname "$unit")"
sed "s|/usr/bin/hito-server|$bin|" "$here/hito-server.service" >"$unit.new"
chmod 644 "$unit.new"
mv -f "$unit.new" "$unit"

version="$("$bin" --version)"
if [ "$systemd" = 1 ]; then
  systemctl daemon-reload
  systemctl enable hito-server.service >/dev/null 2>&1
  systemctl restart hito-server.service
  echo "Installed $version and started the hito-server service."
  echo "Check it:  curl http://127.0.0.1:7878/healthz   (answers ok)"
  echo "Logs:      journalctl -u hito-server -f"
else
  echo "Installed $version. systemd is not running here, so start it yourself:"
  echo "  su -s /bin/sh hito -c '$bin --config /etc/hito/hito.toml'"
fi
echo "Next: put a TLS proxy in front and set the address in /etc/hito/hito.toml (see README.md)."
