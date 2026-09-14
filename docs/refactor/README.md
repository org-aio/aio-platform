# 历史整改与验收记录

以下记录保留当时的提交和验收范围，其中“未上线”“待迁移”仅描述对应历史阶段。当前模块职责与本地沙箱入口以 [开发文档](../development/README.md) 为准：通用宿主和发布队列已经归平台，产品继续装配静态系统插件；现役包协议可用，本轮不强制迁移全部业务插件。

# Fullstack Runtime Cutover

## Repository Ownership

| Repository | Ownership |
| --- | --- |
| aio-platform | Runtime, WIT, capability proxies, SDK, CLI, package validation |
| aio-idea | Product startup, deployment and default composition |
| aio-plugin-identity | Identity, sessions, users, RBAC, tenants, account surfaces and settings |
| aio-plugin-marketplace | Marketplace, contribution review and composition UI |
| aio-plugin-dictionary / aio-plugin-file | Complete independent business applications |
| aio-plugin-studio | Editor, definitions, compiler, generated applications and business implementations |
| aio-plugin-*-example | One fullstack reference repository per language |

`frontend/`, `backend/`, `shared/` are internal modules, not separate release units. Workbench crates are libraries. Dill and TypeId operate only inside one Rust process; source UUIDs, package digests and page IDs have separate transport and navigation meanings.

## Verified In This Cutover

- Local directories renamed from `aio` / `aio-public-shell` to `aio-platform` / `aio-idea`; their existing Git remotes already use the target names. Studio is an independent local repository. The fullstack KMP example is published at `zjarlin/aio-plugin-kmp-example`; no old repositories have been archived.
- Studio application, migrations, generated business implementations and filtered Git history preserved in the independent local Studio repository; removed from platform workspace.
- Platform dependency boundary check in CI. Product dependency check intentionally still fails until its static system plugins are migrated.
- Canonical `lib/plugin/contract/wit/plugin.wit`, package `aio:plugin@2.0.0`: typed page entry metadata, binary request/response, context, database, storage and management interfaces.
- Wasmtime component execution in platform crate, host capability grants, transaction cleanup, isolated schema/data roles and bounded object namespace.
- Kotlin Toolchain builds actual Compose wasmJs frontend and Kotlin wasmWasi Component in one example repository.
- Real Kotlin Component integration test verifies PostgreSQL persistence across instance replacement, tenant separation and capability denial.
- PostgreSQL integration tests also verify native role isolation, denied DDL/role switching, rollback on request cleanup and rollback after lock timeout.
- Null parameters infer PostgreSQL column types, including boolean, integer, real, text, binary and JSON. Cancelling a Component call drops its execution state and uncommitted transactions even if the supervisor retains the invalid instance handle.
- Desktop 1280x800 and mobile 390x844 browser tests verify actual Compose canvas, `+1` backend calls, pixel changes, persistence after reload and zero console errors.

The latest browser run used a separate loopback preview instance to avoid concurrent clicks in the user-visible preview. Desktop count changed from 0 to 1 (483 changed canvas pixels); mobile count changed from 1 to 2 (387 changed pixels). Both reloaded values persisted and both console error lists were empty. The extra test server was stopped; the user preview remains at `http://127.0.0.1:4187/`.

## Still Required Before Production Cutover

- Wire the production publish API to the v2 persistent registry; no v1 adapter in the new runtime. `PersistentComponentSlot` now persists bundles, grants, activation history and active revisions, restores after restart and fences stale replicas. The public install API has not migrated to it.
- Remove the 14 remaining static business Cargo dependencies from `aio-idea` after their replacement plugins pass acceptance. Do not remove the dependencies prematurely and replace functioning system pages with placeholders.
- Persistent encrypted database bindings and checksummed migration history are implemented and tested. Complete production wiring, broader controlled upgrades, data-compatible rollback and aggregate quotas; the current migration policy only permits additive tables/indexes and refuses changed/deleted migration history.
- Complete signed/revocable mount tickets, context/theme/navigation/fullscreen/account protocols and bootstrap identity provider.
- Migrate and merge identity/RBAC/tenant/account/settings, then marketplace, dictionary, file and Studio onto actual v2 Components. Studio code extraction is not its runtime migration.
- Move source assembly to `aio host extension`; rebuild thin-host and fullstack CLI templates and examples. Existing CLI packaging still targets the old production protocol; do not publish v2 artifacts with it yet.
- Consolidate all examples and archive retired repositories only after replacement verification.
- Database-copy rehearsal, backup, maintenance cutover and full production acceptance on 252.

The local preview is an explicit loopback-only development runner, not a deployed product. The current user preview at `http://127.0.0.1:4187/` now runs the Compose + Ktor mock workbench; the Component remains a separate build target in the same repository.

The public product was updated to `5e7670f46cdc2052f7445a718acbfdbd76c75999` for complete Compose asset paths, bounded fullstack packages and the JSON bridge. This is not the v2 cutover: the existing process executor runs the Ktor target with an external pinned JRE image, and the v2 Component publication gate remains closed. The JAR does not contain a JVM, and Ktor mock state is not PostgreSQL persistence.

## Ktor Public Acceptance

On 2026-09-11, package `0.2.0` from `zjarlin/aio-plugin-kmp-example` was activated in tenant `default`, under Community / KMP fullstack example. Source commit: `ed8a2674b426f49abf31c57a71988cd158545995`; package digest: `4fcdcd1de7f2b706b3a2fdd406d2c05406a9966022b8ce3eec1bd8a381e682c8`.

- Real Compose + Ktor task create/update/delete, cancellation of delete, search, counter, reload/re-entry, sandbox isolation and desktop/mobile screenshots passed. Console error lists were empty.
- Shell PID stayed `15349` across installation and bad-package rejection. Executable and frontend index SHA256 stayed unchanged; the bad package returned HTTP 400 and preserved the active revision.
- Backend JAR: 11.04 MiB; frontend assets: 26.49 MiB; compressed fullstack package: 20.40 MiB. The separate Ktor container used about 73 MiB after startup, under a 256 MiB cap. JRE image layers are external to the package.
- GitHub CI built both Compose and Ktor from the independent repository successfully.
- The public upload response timed out at the proxy, but subsequent catalog inspection and browser acceptance confirmed successful activation. Upload receipt/recovery still needs improvement; do not report that request as an uninterrupted successful upload.
- Compose beta clears some accessibility nodes after closing dialogs. Browser tests calibrate their bounds and send real pointer events to the canvas; these results do not constitute screen-reader acceptance.

The broader v2 system-plugin cutover and destructive lifecycle/data migration acceptance remain outstanding.

## Dialog Memory Integration

Agent and Agent Memory now exercise the real Component/process boundary through an authenticated loopback development broker: encrypted receipt and idempotency, sanitized history and recall, independent secret grants, durable compilation leases, foreground preemption, source deletion visibility, source clarification, wiki revisions/aliases/relations, conflict review and rollback. A canary secret is checked across model requests, ordinary messages, context/search/graph and logs. Desktop/mobile Compose chat and protected reveal/copy were verified with an inspectable SSE test endpoint, not a live model quality evaluation.

The runtime also has versioned host cryptography, durable scoped database credentials and persistent Component activation. PostgreSQL tests verify additive migration/restart, invalid history/key rejection, candidate failure preservation, restart restoration and rejection of stale revisions across execution slots. These libraries do not by themselves migrate `aio-idea` identity/navigation/publication or grant process egress.

The public Agent release remains blocked by the existing production cutover: controlled process model egress and cross-plugin broker, host key injection, native v2 publication/mount identity, system-plugin migration and database-copy acceptance on 252. No new business Cargo dependency or old-protocol adapter was added, and no public v2 package was installed.

## Local Compose Interaction And Asset Delivery

On 2026-09-11, Ktor package `0.2.1` was activated in both `default` and `71d108ca-af94-4c4c-917e-050e5a730bde`. Source commit: `080cf72741b6a86e61755b6132c6320affa1cd1b`; package digest: `bd1fb4dfa22b7dad6f9317a32e2d1a757a85d5dfcc94ddf36c111526f08fa9a0`.

- Workbench Counter now uses local Compose state, sends no counter requests, retains its value across tabs and resets when the plugin reloads. Tasks still performs real Ktor mock CRUD. The separate `counter.html` Component database example remains separate and unchanged in behavior.
- Public desktop 1280x800 and mobile 390x844 acceptance with the user's tenant passed: 20 offline increments each, zero counter requests, actual canvas repaint, tab retention, reload reset, task CRUD, sandbox isolation, no horizontal overflow and no console errors. Test-created tasks were deleted. Browser tests resolve live counter button bounds; document querySelector alone cannot inspect Compose's shadow-root canvas.
- The plugin update itself preserved host PID `15349`, executable SHA256 `2959f9c22a895aa327b99ac30ad9560d568fb65f9b367c5536ccd78fd8824770`, and frontend index SHA256 `b94f0e114840ac660156e9e796c1818664d48282dd8b9a8eb46611ccafe34f47`.
- A separate host-only delivery fix `4bdcc78e9d7d5f76cb452a6b61a132f0fe3cad84` enabled negotiated gzip for static plugin code, retaining CSP, authorization and `private, no-store, no-transform`. This infrastructure update restarted the host to PID `13922`, preserved the existing frontend shell and retained release `5e7670f46cdc2052f7445a718acbfdbd76c75999` for recovery. It was not required by the plugin upgrade.
- Public responses now carry `Content-Encoding: gzip`. The two browser Wasm assets are about 23.25 MB uncompressed and 6.85 MB gzipped. Loading/error status is provided by the plugin. Before the fix, an observed Skia download took 94 seconds; later compressed samples took 10-14 seconds, with different network routes, so these timings are observations rather than a controlled performance comparison.
- Host tests: 47 regular tests, one additional isolated PostgreSQL frontend HTTP test, and Clippy passed. The production plugin source also passed GitHub Compose/Ktor CI. This remains the existing process protocol, not the v2 production cutover.

## v2 Bundle And Execution Slot

`az-plugin-bundle` now packages prebuilt backend Component, complete frontend assets and ordered SQL migrations into one deterministic gzip/JSON bundle. It requires a full Git SHA, binds all content and provenance to SHA256, and rejects undeclared files, unsafe paths, symbolic links, missing artifacts, oversized content, core browser Wasm/JAR backends and archive trailers. It does not execute scripts or establish author trust by digest alone.

The v2 manifest no longer requires `runtime.kind`; this Component path validates the artifact itself. Page/navigation metadata comes from the WIT description, not duplicated subplugin page declarations. Ktor remains a separate, externally supervised process target with no bundled JVM; the production process format is not a v2 adapter.

`ComponentSlot` validates the package and grants, prepares and checks a candidate while the previous release serves traffic, then drains requests and switches assets/description/backend together. Old page digests cannot invoke the new backend. Changed migration sets are refused online. This is a process-local mechanism, not the production registry transaction, identity bootstrap or maintenance migrator.

Tests cover failure preservation, cancelled preparation, request draining, binary responses, revoked revisions and environment isolation with real Rust-generated Components. The Kotlin fullstack bundle integration uses real PostgreSQL and checks preserved data after replacement/reactivation plus tenant and capability isolation. Existing Compose/Ktor browser acceptance remains separate; these backend changes do not imply a new public deployment.
