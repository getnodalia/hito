#!/bin/sh
# hito-server .deb/.rpm pre-remove: stop and disable the service on removal (not on upgrade).
# The data in /var/lib/hito and the configuration in /etc/hito stay.
case "${1:-}" in
  remove | purge | 0)
    if [ -d /run/systemd/system ]; then
      systemctl disable --now hito-server.service >/dev/null 2>&1 || true
    fi ;;
esac
exit 0
