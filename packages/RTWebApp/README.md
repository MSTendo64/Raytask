# RTWebApp

`RTWebApp` is a RayTask-first helper library for server-side applications built on top of:

- `bstd.web`
- `bstd.sqlite`
- `bstd.json`
- `bstd.crypto`

It keeps the runtime-native layer thin and moves reusable app/web logic into ordinary RayTask code.

## What it provides

- SQL-safe string escaping helpers
- JSON-safe template model quoting
- slug generation
- page rendering helpers for `Template.Render(...)`
- `RTWebContext` request wrapper
- `RTWebRouteMatch` prefix-based route abstraction
- auth/session helpers
- admin bootstrap helpers
- audit log helpers
- token-based publish helper logic for registry-style apps

## Import

After installing from a registry:

```rt
import "external/RTWebApp/src/lib";
```

From this repository:

```rt
import "../../../packages/RTWebApp/src/lib";
```

## Example

```rt
import bstd.web;
import "../../../packages/RTWebApp/src/lib";

void Main() {
    var ctx = new RTWebContext();
    if (ctx.Is("GET", "/")) {
        RTWebRenderPage(
            "apps/registry/templates/layout.html",
            "Demo",
            "<section><h1>Hello from RTWebApp</h1></section>",
            "<nav><a href=\"/\">Home</a></nav>",
            ""
        );
        return;
    }

    var pkg = ctx.MatchPrefix("GET", "/packages/");
    if (pkg.Matched && pkg.Count() == 1) {
        Web.Text("package = " + pkg.Part(0));
        return;
    }

    Web.SetStatus(404);
    Web.Text("not found");
}
```

## Package metadata

The package manifest lives at `packages/RTWebApp/package.rtp`, so the folder is already registry-ready.
