# Hito server

Hito's server is one program and one SQLite file. This folder installs it on a Linux machine
with systemd: a Debian or Ubuntu server, a Proxmox LXC container, a Raspberry Pi with a 64-bit
system, a small VPS.

## Install

```sh
tar xzf hito-server-*-linux-*.tar.gz
cd hito-server-*-linux-*/
sudo ./install.sh
curl http://127.0.0.1:7878/healthz    # answers ok
```

`install.sh` puts the program in `/usr/local/bin/hito-server`, creates the `hito` system user
and its data folder `/var/lib/hito`, writes `/etc/hito/hito.toml` (only if it isn't there),
and starts the systemd service `hito-server`. Running it again upgrades the program and keeps
your configuration and data. `sudo ./install.sh --uninstall` removes the program and the
service, and keeps the data.

On Debian and Ubuntu the `hito-server` `.deb` does the same through the package manager
(`sudo apt install ./hito-server_*.deb`), and on Fedora the `.rpm`.

## Make it reachable

The server starts on `127.0.0.1:7878`. Put a reverse proxy with TLS in front, then edit
`/etc/hito/hito.toml` and `sudo systemctl restart hito-server`.

**Caddy on the same machine** (it gets the certificate by itself). `/etc/caddy/Caddyfile`:

```
chat.example.org {
    reverse_proxy 127.0.0.1:7878
}
```

and `trust_proxy = true` in `hito.toml`.

**A proxy on another machine** (Nginx Proxy Manager, Traefik, a proxy in another container):
in `hito.toml` set `bind = "0.0.0.0:7878"` and `trust_proxy = true`, point the proxy at
`http://<this machine's IP>:7878`, and turn on WebSockets for it (live messages use
`/api/v1/ws`). Then only the proxy should reach port 7878.

On a new installation, run `sudo -u hito hito-server --config /etc/hito/hito.toml init-token`.
Enter the token in the invitation code field to create the first account within 15 minutes.
Existing installations with users do not need this step. In the desktop app, the server is
`https://chat.example.org`. With `registration = "invite_only"` subsequent accounts need an
invitation from a workspace owner.

## Proxmox

1. Create a container from the Debian 12 template: unprivileged, 1 core, 512 MB of memory, 4 GB
   of disk, a fixed IP address.
2. In its console, as root:
   ```sh
   apt update && apt install -y curl ca-certificates
   curl -LO https://github.com/getnodalia/hito/releases/latest/download/hito-server-linux-x86_64.tar.gz
   tar xzf hito-server-linux-x86_64.tar.gz && cd hito-server-*-linux-x86_64/ && ./install.sh
   ```
3. Make it reachable as above.
4. Back it up with Proxmox's own backups of the container, or with
   `sqlite3 /var/lib/hito/hito.db ".backup /root/hito.db"`.

## Day to day

| What | How |
| --- | --- |
| Logs | `journalctl -u hito-server -f` |
| Restart after a configuration change | `sudo systemctl restart hito-server` |
| Upgrade | unpack the new release and run `sudo ./install.sh` again (migrations run on start) |
| Everything else | https://github.com/getnodalia/hito/blob/main/docs/deployment.md |
