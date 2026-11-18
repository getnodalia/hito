#!/bin/sh
# hito-server .deb/.rpm post-remove: forget the removed unit.
if [ -d /run/systemd/system ]; then
  systemctl daemon-reload >/dev/null 2>&1 || true
fi
exit 0
