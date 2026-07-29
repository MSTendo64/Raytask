# RayTask Registry Protocol

This document specifies what the RayTask package manager (`raytask install / search`) expects from a registry server and how to host one yourself.

---

## Overview

A RayTask registry is a simple HTTP server (or a local directory) that serves:

| Endpoint | Method | Description |
|---|---|---|
| `/index.json` | GET | Full catalog of available packages |
| `/packages/{name}/{version}.zip` | GET | Package archive |

No authentication is required by default. For private registries, add a `token:` field in `rtp.repos.yml` — the token is sent as `Authorization: Bearer <token>`.

---

## `GET /index.json`

The server must respond with `Content-Type: application/json` and the following schema:

```json
{
  "registry": "Official RayTask Registry",
  "packages": [
    {
      "name": "HttpClient",
      "version": "1.2.0",
      "versions": ["1.2.0", "1.1.0", "1.0.3"],
      "description": "HTTP/1.1 and HTTP/2 client library for RayTask",
      "instructions": "Import with:\n  import \"external/HttpClient/src/lib\"\n\nSet base URL:\n  let c = Http.NewClient(\"https://api.example.com\");",
      "author": "Jane Doe <jane@example.com>",
      "homepage": "https://github.com/example/raytask-httpclient",
      "license": "MIT",
      "tags": ["http", "web", "networking"],
      "download_url": null
    }
  ]
}
```

### Field reference

| Field | Type | Required | Notes |
|---|---|---|---|
| `name` | string | ✅ | Case-sensitive package identifier. |
| `version` | string | ✅ | Latest stable version (semver). |
| `versions` | string[] | — | All available versions, newest first. If omitted, only `version` is available. |
| `description` | string | — | One-line summary shown in `raytask search`. |
| `instructions` | string | — | Full install/usage guide shown with `raytask install --info`. May contain Markdown. |
| `author` | string | — | Maintainer name / email. |
| `homepage` | string | — | URL to project page / docs. |
| `license` | string | — | SPDX license identifier (e.g. `MIT`, `Apache-2.0`). |
| `tags` | string[] | — | Keywords for search. |
| `download_url` | string | — | Explicit URL to the archive. Overrides the default pattern. |

---

## `GET /packages/{name}/{version}.zip`

This is the **default download URL pattern** the client uses when `download_url` is not set in the index.

### Archive format

The `.zip` archive **must** be a standard ZIP file. Both **Store** (method 0) and **Deflate** (method 8) compression methods are supported.

#### Recommended package layout

```
HttpClient/
├── package.rtp          # Package manifest (required)
├── src/
│   └── lib.rt           # Main export file
├── examples/
│   └── demo.rt
└── README.md
```

#### `package.rtp` manifest inside the archive

```
package "HttpClient" {
    version = "1.2.0"
    author  = "Jane Doe"
    description = "HTTP client for RayTask"

    export {
    }
}
```

After installation the package lands at `external/HttpClient/` and its lock file is written to `external/HttpClient/rtp.lock.yml`.

---

## Version resolution rules

1. If the user specifies an exact version (`raytask install HttpClient@1.1.0`), only that version is accepted.
2. If no version is given, the **newest version** across all repositories is selected (lexicographic semver comparison).
3. When the same package version exists in multiple repositories, the one with the **highest `priority`** in `rtp.repos.yml` wins.
4. Repositories are tried in descending priority order; the first repo that has the requested version wins (for same-priority repos, declaration order in the YAML is used).

---

## Local file registry

A local directory can act as a registry. Set `url: file:///path/to/registry` (or a bare path) in `rtp.repos.yml`. The directory must contain:

```
registry/
├── index.json           # Same format as above
└── packages/
    └── HttpClient/
        └── 1.2.0.zip
```

---

## Hosting a minimal registry server

Below is a minimal example using Node.js + Express:

```js
import express from 'express';
import fs from 'fs';
import path from 'path';

const app = express();
const BASE = './registry';

app.get('/index.json', (_req, res) => {
  res.sendFile(path.resolve(BASE, 'index.json'));
});

app.get('/packages/:name/:version', (req, res) => {
  const { name, version } = req.params;
  const file = path.join(BASE, 'packages', name, version);
  if (fs.existsSync(file)) return res.sendFile(path.resolve(file));
  res.status(404).json({ error: 'not found' });
});

app.listen(8080, () => console.log('Registry running on :8080'));
```

Point `rtp.repos.yml` at it:

```yaml
repositories:
  - name: local-server
    url: http://localhost:8080
    priority: 10
```

---

## `rtp.repos.yml` reference

```yaml
repositories:
  - name: official          # Human-readable label
    url: https://registry.raytask.dev
    priority: 100           # Higher = preferred (default 0)
    secure: true            # Reject http:// (default false)
    token: "s3cr3t"         # Sent as: Authorization: Bearer s3cr3t

install_dir: external       # Where packages are installed (default: external/)
```

Global config: `~/.raytask/rtp.repos.yml` (Windows: `%USERPROFILE%\.raytask\rtp.repos.yml`).  
Project config: `rtp.repos.yml` in the project root (takes precedence if both exist... currently first match wins).

---

## CLI reference

```
raytask install <Name>              # Install latest version
raytask install <Name>@<version>    # Install specific version
raytask install <Name> --info       # Show description + instructions, then confirm
raytask uninstall <Name>            # Remove from external/
raytask search <query>              # Search all configured repos
raytask list                        # List installed packages
raytask update                      # Reinstall dependencies from project.rtp
```

---

## Importing installed packages

After `raytask install HttpClient`, use:

```rt
import "external/HttpClient/src/lib"

func main() {
    let client = Http.NewClient("https://httpbin.org");
    let resp = client.Get("/get");
    print_ln(resp);
}
```

Or declare the dependency in `project.rtp` so `raytask update` restores it automatically:

```
project "MyApp" {
    version = "1.0.0"

    dependencies {
        "HttpClient" = "1.2.0"
    }
}
```
