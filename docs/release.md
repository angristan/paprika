# Release, smoke test, and rollback

Paprika is a static Cloudflare Worker Assets deployment. `web/dist/` is the complete release payload; there is no Worker script, runtime binding, or server-side document processing.

## Release gate

Run from a clean checkout:

```bash
bun install --frozen-lockfile
bash scripts/install-rust-toolchain.sh
bash scripts/install-wasm-pack.sh "$HOME/.cache/paprika-tools/bin"
export PATH="$HOME/.cache/paprika-tools/bin:$PATH"
bun run predeploy
bun run test:e2e
```

`predeploy` runs locked Rust formatting, Clippy, tests, the wasm target check, JavaScript checks, a pinned EPUBCheck regression, the production WebAssembly build, deploy-file validation, and `wrangler deploy --dry-run`. Browser E2E starts a new Wrangler server on loopback; it never targets production or a branch preview.

The EPUB check requires Java 17 or newer. It downloads EPUBCheck 5.3.0 and the public QuiCK regression PDF into the local cache, and verifies both pinned SHA-256 digests before execution.

Because Cloudflare Workers Builds deploys every accepted `main` commit, the GitHub `main` branch protection rule is part of the production gate. It must require these current workflow checks before merge:

- `Locked build and smoke tests`
- `Native tests (ubuntu-24.04)`
- `Native tests (macos-14)`
- `Native tests (windows-2022)`

Do not merge if the rule is absent, stale, or bypassed. Update the rule whenever workflow job names change.

Pinned external inputs:

| Input | SHA-256 |
| --- | --- |
| rustup-init 1.28.2, Linux x86-64 GNU | `20a06e644b0d9bd2fbdbfd52d42540bdde820ea7df86e92e533c073da0cdd43c` |
| wasm-pack 0.15.0, Linux x86-64 musl archive | `c09f971ecaed9a2efc80fdcea7a00ef6b53c7fadc8c57d1f61b53a6aa66b668a` |
| EPUBCheck 5.3.0 archive | `6c07e68584b2e2ce2f89fe06e1246dfead3eb36b46b340e7d93524f29dcff6c5` |
| FoundationDB QuiCK PDF fixture | `90b16b703c680aa90291d6008cdaadeaa7d604a3889ee5d3bb347db4c81a06db` |

The first three values match publisher-hosted checksum metadata or GitHub release-asset digests. The QuiCK site does not publish a checksum; its value pins the reviewed bytes downloaded from the documented URL and detects any later change.

## Workers Builds settings

Use these exact dashboard settings:

- Production branch: `main`
- Build command: `bun run build:cloudflare`
- Deploy command: `bun run deploy`
- Root directory: repository root

The deploy command intentionally does not rebuild. It uploads the `web/dist/` payload already checked by the build command. Do not replace it with `bun run predeploy`, `bun run build`, or a chained build-and-deploy command.

Non-production branch builds, if enabled, must remain preview deployments. Pull-request browser tests use only `127.0.0.1` and do not depend on a preview URL or the production hostname.

Before release, record the current successful production deployment identifier from the Cloudflare dashboard. After the approved change reaches `main`, wait for Workers Builds to finish and record the new identifier.

## Production smoke test

Check the public site without uploading a private document:

```bash
origin=https://paprika.stanislas.cloud
curl --fail --silent --show-error --head "$origin/"
curl --fail --silent --show-error "$origin/LICENSE-APACHE" >/dev/null
curl --fail --silent --show-error "$origin/LICENSE-MIT" >/dev/null
curl --fail --silent --show-error "$origin/pkg/paprika_wasm_bg.wasm" >/dev/null
```

Then use a small, non-sensitive born-digital PDF in a normal browser:

1. Confirm the page identifies local PDF-to-EPUB conversion and shows no failed asset requests.
2. Select the PDF and confirm the source preview, file name, size, and enabled **Make EPUB** action.
3. Convert it. Confirm progress completes, an EPUB preview appears, and the download is non-empty.
4. Open the EPUB in an independent reader. Confirm text is selectable and source-page order is plausible.
5. In browser developer tools, confirm conversion makes no request containing document bytes and no request to a third-party origin.
6. Select raster PDF and confirm the UI labels it experimental before starting work.

A release is not healthy if the HTML succeeds but the WebAssembly module, worker, security headers, licenses, conversion, preview, or download fails.

## Rollback

Rollback is an explicit production mutation. Get approval before starting it.

1. Stop further release attempts and preserve the failed deployment/build logs.
2. In the Cloudflare dashboard, open the Paprika Worker deployment history.
3. Select the recorded last-known-good production deployment and use the dashboard rollback action if it is available for that asset deployment.
4. If an asset rollback is not offered, restore the last-known-good source through the normal reviewed change process and let Workers Builds create a new deployment. Do not bypass the release gate or manually upload a local `web/dist/` directory.
5. Repeat the production smoke test and record the active deployment identifier.
6. Fix forward in a separate reviewed change with a regression test.

Do not delete deployment history, project settings, routes, or domains as part of rollback.
