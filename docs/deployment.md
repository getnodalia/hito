# Running a server

Hito's server is one binary and one SQLite file. Run it on loopback and let a reverse proxy
handle TLS. Nothing in Hito talks to an outside service.

## Install

Each [release](https://github.com/getnodalia/hito/releases/latest) has the server on its own,
apart from the desktop app, with no desktop libraries:

| File | For |
| --- | --- |
| `hito-server-<version>-linux-x86_64.tar.gz` | any Linux with systemd: unpack it and run `sudo ./install.sh` |
| `hito-server-<version>-linux-x86_64.deb` | Debian, Ubuntu, a Proxmox LXC: `sudo apt install ./hito-server-*.deb` |
| `hito-server-<version>-linux-x86_64.rpm` | Fedora, RHEL: `sudo dnf install ./hito-server-*.rpm` |

All three do the same: the program, the `hito` system user, the data folder `/var/lib/hito`,
the configuration `/etc/hito/hito.toml` (kept on upgrades) and the systemd service
`hito-server`, enabled and started on loopback. Check it with
`curl http://127.0.0.1:7879/healthz`, which answers `ok`. The tarball's `README.md` walks through
it, and `sudo ./install.sh --uninstall` removes the program and the service and keeps the data.

`releases/latest/download/hito-server-linux-x86_64.tar.gz` always points at the newest one.
The macOS and Windows downloads carry `hito-server` too. Or build it:

```sh
cargo build --release -p hito-server
```

## Proxmox

A small unprivileged container is enough: the Debian 12 template, 1 core, 512 MB of memory and
4 GB of disk, with a fixed address. In its console, as root:

```sh
apt update && apt install -y curl ca-certificates
curl -LO https://github.com/getnodalia/hito/releases/latest/download/hito-server-linux-x86_64.tar.gz
tar xzf hito-server-linux-x86_64.tar.gz && cd hito-server-*-linux-x86_64/ && ./install.sh
```

Then make it reachable with TLS as below. With the reverse proxy in another container or machine
(Nginx Proxy Manager, Traefik...), set `bind = "0.0.0.0:7879"` and `trust_proxy = true` in
`/etc/hito/hito.toml`, `systemctl restart hito-server`, point the proxy at
`http://<container IP>:7879` with WebSockets on, and let only the proxy reach that port.
Proxmox's own container backups cover the database.

## Configure

`/etc/hito/hito.toml`, then `sudo systemctl restart hito-server`:

```toml
name = "Our team"
bind = "127.0.0.1:7879"
database = "sqlite:///var/lib/hito/hito.db"
registration = "invite_only"   # subsequent accounts require invitations
trust_proxy = true             # the proxy below terminates TLS
```

## Initialize a new server

An empty installation needs a one-use token before the first account can register. On the server,
using the same configuration and the service account that owns the database, run:

```sh
sudo -u hito hito-server --config /etc/hito/hito.toml init-token
```

Enter the displayed token in the desktop registration screen's invitation code field within
15 minutes. Treat it like a password. Running the command again invalidates the previous token.
Once the first account is created, initialization cannot be repeated. Existing installations
with accounts are marked initialized during migration and do not need a token. Back up the SQLite
database before upgrading, and test the upgrade on a copy first.

## TLS with Caddy

Caddy gets and renews a certificate by itself. `/etc/caddy/Caddyfile`:

```
hito.example.org {
    reverse_proxy 127.0.0.1:7879
}
```

WebSockets pass through `reverse_proxy` without extra settings. With nginx, forward
`Upgrade`/`Connection` headers on `/api/v1/ws` and set `X-Forwarded-For`.

## Run as a service

The packages and `install.sh` install the unit
([`packaging/linux/server/hito-server.service`](../packaging/linux/server/hito-server.service)):
it runs as `hito`, can write only to `/var/lib/hito`, and restarts on failure.

```sh
sudo systemctl status hito-server
journalctl -u hito-server -f
```

In the desktop app, the server is `https://hito.example.org`.

## Backups

The database is `hito.db` with its `-wal` file. Back up with SQLite's online backup, which is
safe while the server runs:

```sh
sqlite3 /var/lib/hito/hito.db ".backup /var/backups/hito-$(date +%F).db"
```

Backups hold accounts, metadata (projects, statuses, dates, who is assigned, the activity log) and
the ciphertext of what the team wrote: store them as carefully as the server.

The server can also create a consistent snapshot without a separate SQLite program. Use a
private directory writable by the service account; the destination must not already exist:

```sh
sudo -u hito hito-server --config /etc/hito/hito.toml backup /var/lib/hito/backup.db
```

The command checks the copy's integrity and does not apply migrations to the source. The copy
contains sensitive data and needs independent protection and a tested restore procedure.

## Upgrading

The server updates itself. The `.deb`, the `.rpm` and `install.sh` enable the systemd timer
`hito-server-update.timer`, which once a day (around 04:00, spread over two hours) runs
`hito-server self-update --auto` as root. It:

1. reads the newest stable release from `github.com/getnodalia/hito/releases/latest` (a web
   redirect, so GitHub's limit on anonymous API calls does not apply);
2. stops if it is not newer than the installed version;
3. downloads the package for this installation (`.deb`, `.rpm`, or the `.tar.gz` that
   `install.sh` came from) and checks it against the release's `SHA256SUMS`;
4. snapshots the database, and the 2FA key file beside it, to `backups/before-update-*.db` next to
   the database (the last three are kept);
5. installs it (`dpkg -i`, `rpm -U`, or a swap of the binary that keeps the old one as
   `hito-server.previous`) and restarts the service;
6. waits for `/healthz`. A `.tar.gz` install whose new version does not answer gets the previous
   binary back.

See what it did with `journalctl -u hito-server-update`. To update by hand instead, set
`automatic = false` under `[updates]` in `/etc/hito/hito.toml` (or run
`sudo systemctl disable --now hito-server-update.timer`), and run
`sudo hito-server --config /etc/hito/hito.toml self-update` when you choose. Installing the new
release the way you installed the old one (`./install.sh` again, or the new package) also works.
Migrations run on start and only move forward.

To check whether a newer stable release exists without changing the installation, run
`hito-server check-updates`. This command queries the public GitHub releases API over HTTPS;
it does not start the server, access the database, download an executable or install updates.
It requires network access and can fail if GitHub is unreachable or rate-limits the request.
