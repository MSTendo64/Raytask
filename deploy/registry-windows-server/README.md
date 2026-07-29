# RayTask lib Registry - Windows Server Copy

This deployment copy runs the shared `apps/registry/` application with Windows-oriented scripts and paths.

## Expected layout

- application code: `apps/registry/`
- reusable framework package: `packages/RTWebApp/`
- data root: `deploy/registry-windows-server/data/`

## Run

```cmd
deploy\registry-windows-server\run.cmd
```

## Optional env vars

- `RAYTASK_REGISTRY_HOST`
- `RAYTASK_REGISTRY_PORT`
- `RAYTASK_REGISTRY_ADMIN_USER`
- `RAYTASK_REGISTRY_ADMIN_PASS`
- `RAYTASK_REGISTRY_PUBLISH_TOKEN`
