# RayTask lib Registry - Linux Server Copy

This deployment copy runs the shared `apps/registry/` application with Linux-oriented scripts and service metadata.

## Expected layout

- application code: `apps/registry/`
- reusable framework package: `packages/RTWebApp/`
- data root: `deploy/registry-linux-server/data/`

## Run

```bash
chmod +x deploy/registry-linux-server/run.sh
./deploy/registry-linux-server/run.sh
```

## Optional env vars

- `RAYTASK_REGISTRY_HOST`
- `RAYTASK_REGISTRY_PORT`
- `RAYTASK_REGISTRY_ADMIN_USER`
- `RAYTASK_REGISTRY_ADMIN_PASS`
- `RAYTASK_REGISTRY_PUBLISH_TOKEN`
