# Deployment notes

## GHCR image

The GitHub Actions workflow builds and pushes:

- `ghcr.io/<owner>/inkstone:latest`
- `ghcr.io/<owner>/inkstone:sha-<short>`
- `ghcr.io/<owner>/inkstone:vX.Y.Z` (tag builds)

If the registry is private, log in on the server:

```bash
podman login ghcr.io -u <user> -p <token>
```

## Quadlet (systemd)

1) Copy `deploy/systemd/inkstone.container` to `/etc/containers/systemd/`.
2) Create `/opt/inkstone/.env` with your runtime settings.
3) Ensure `/opt/inkstone/data` is writable by uid `10001`.
4) Reload and start:

```bash
sudo systemctl daemon-reload
sudo systemctl restart inkstone.service
```

Quadlet reads `inkstone.container` and generates `inkstone.service` under
`/run/systemd/generator/`. That generated unit cannot be enabled; just start or
restart it after changes. The `[Install]` section in the `.container` file is
applied by the generator on boot.

If you want rootless podman, use `~/.config/containers/systemd/` and:

```bash
systemctl --user daemon-reload
systemctl --user restart inkstone.service
```

### Auto update (podman auto-update)

The quadlet file enables auto-update labels. Turn on the built-in timer:

```bash
sudo systemctl enable --now podman-auto-update.timer
```

### Manual update

If you want to update manually (same logic as the timer):

```bash
sudo systemctl start podman-auto-update.service
```

## Podman compose (optional)

1) Copy `deploy/docker/docker-compose.yml` to your server (e.g. `/opt/inkstone/`).
2) Replace `OWNER` with your GitHub org/user.
3) Create `/opt/inkstone/.env` with your runtime settings.

Run:

```bash
cd /opt/inkstone
podman compose up -d
```

### Data directory and index path

The container runs as uid `10001`, so the host data directory must be writable by that user.
If you mount `/opt/inkstone/data` to `/data`, set:

```
INKSTONE_INDEX_DIR=/data/index
```

Ensure permissions:

```bash
sudo mkdir -p /opt/inkstone/data
sudo chown -R 10001:0 /opt/inkstone/data
sudo chmod -R u+rwX,g+rwX /opt/inkstone/data
```

If you use rootless podman, run the ownership change via:

```bash
podman unshare chown -R 10001:0 /opt/inkstone/data
```
