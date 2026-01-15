# Deployment notes

## GHCR image

The GitHub Actions workflow builds and pushes:

- `ghcr.io/<owner>/inkstone:latest`
- `ghcr.io/<owner>/inkstone:sha-<short>`
- `ghcr.io/<owner>/inkstone:vX.Y.Z` (tag builds)

If you need dedicated public/admin images, build with:

```bash
docker build -f deploy/docker/Dockerfile --build-arg INKSTONE_PACKAGE=public -t ghcr.io/<owner>/inkstone-public:latest .
docker build -f deploy/docker/Dockerfile --build-arg INKSTONE_PACKAGE=admin -t ghcr.io/<owner>/inkstone-admin:latest .
```

If the registry is private, log in on the server:

```bash
podman login ghcr.io -u <user> -p <token>
```

## Quadlet (systemd)

1) Copy `deploy/systemd/inkstone-public.container` and `deploy/systemd/inkstone-admin.container`
   to `/etc/containers/systemd/`.
2) Create `/opt/inkstone/.env` with your runtime settings.
3) Ensure `/opt/inkstone/data` is writable by uid `10001`.
4) Reload and start:

```bash
sudo systemctl daemon-reload
sudo systemctl restart inkstone-public.service inkstone-admin.service
```

Quadlet reads the `.container` files and generates units under
`/run/systemd/generator/`. Those generated units cannot be enabled; just start or
restart them after changes. The `[Install]` section in the `.container` file is
applied by the generator on boot.

If you want rootless podman, use `~/.config/containers/systemd/` and:

```bash
systemctl --user daemon-reload
systemctl --user restart inkstone-public.service inkstone-admin.service
```

### Auto update (podman auto-update)

The quadlet files enable auto-update labels. Turn on the built-in timer:

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

The admin container runs as uid `10001`, so the host data directory must be writable by that user.
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
