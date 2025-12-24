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

## Podman compose

1) Copy `deploy/docker/docker-compose.yml` to your server (e.g. `/opt/inkstone/`).
2) Replace `OWNER` with your GitHub org/user.
3) Create `/opt/inkstone/.env` with your runtime settings.

Run:

```bash
cd /opt/inkstone
podman compose up -d
```

## Auto update (systemd)

Use the timer to pull and restart periodically:

```bash
sudo cp deploy/systemd/inkstone-update.service /etc/systemd/system/
sudo cp deploy/systemd/inkstone-update.timer /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now inkstone-update.timer
```
