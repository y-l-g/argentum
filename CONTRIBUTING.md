# Contributing to Argentum

Small fixes, documentation corrections, and tests can go straight to a pull request. For a new
feature or a public-API change, open an issue first and describe the problem: redirecting a
design is cheaper than redirecting a patch. Read [`AGENTS.md`](AGENTS.md) before your first
change; it holds the rules this document expands.

## Build and run

```sh
cargo run -p showcase
# open http://localhost:3000/admin/users
```

`crates/argentum-core` is the framework. `examples/showcase` is the runnable admin, the reference
for panel and resource declarations, and the home of the integration tests (`cargo test -p
showcase`); the JavaScript unit tests are `node --test crates/argentum-ui/assets/*.test.js`.

## The gate set

CI runs these nine commands. Run the ones covering your change before pushing,
and all nine before merging.

1. `cargo test --workspace --locked`
2. `cargo clippy --workspace --all-targets --locked -- -D warnings`
3. `cargo check -p argentum-core --no-default-features --locked`
4. `cargo fmt -p argentum-core -p argentum-macros -p argentum-ui -p showcase -p xtask -- --check`
5. `topcoat fmt`, then `git diff --exit-code`
6. `cargo check --locked --manifest-path benchmarks/argentum/Cargo.toml`
7. `cargo clippy --locked --manifest-path benchmarks/argentum/Cargo.toml --all-targets -- -D warnings`
8. `cargo +1.98 check --workspace --locked`
9. `node --test crates/argentum-ui/assets/*.test.js`

Gate 3 keeps the opt-out auth feature compiling: `auth` is on by default in
`argentum-core`, and `default-features = false` stays a working escape hatch
(GH #129). Gate 8 is the MSRV floor declared in `Cargo.toml` (GH #175). CI also
runs `cargo fmt -- --check` inside each detached bench workspace and verifies
that the two lockfiles pin identical `topcoat` and `toasty` revs.

### The `topcoat fmt` trap

The `topcoat` CLI on `PATH` is usually not the revision this workspace locks,
and `topcoat fmt` reflows `view!` markup differently across revisions. CI
installs the CLI at the locked revision before formatting, so a locally
installed CLI of another version proposes a diff CI rejects. Do not hand-fix
that diff. Install the CLI at the locked rev and run it — the exact command is
the `Install topcoat CLI` step of the `fmt` job in
[`.github/workflows/ci.yml`](.github/workflows/ci.yml).

## Vendored primitives

`crates/argentum-ui/src/components/primitives/` mirrors the `topcoat-ui-registry`
crate verbatim, under a `SYNC` header recording the registry version and the
source hash. Never hand-edit those files: update them with
`cargo xtask sync-topcoat-ui`. `cargo xtask verify-topcoat-ui` fails when a
vendored file has drifted, and the xtask test suite runs it on every
`cargo test`. Components Argentum owns live in
`crates/argentum-ui/src/components/composites/` and are edited normally
(ADR-0007).

## Dependency pins

`topcoat` and `toasty` are git dependencies tracking their `main` branches,
pinned to exact commits by `Cargo.lock`. Never run a blanket `cargo update`.
Bump them deliberately:

```sh
cargo update -p topcoat -p toasty
cargo check --offline
```

`cargo check --offline` proves the new revs resolve from the local git cache
instead of failing halfway through a fetch. `benchmarks/argentum` is a detached
workspace with its own lockfile: bump it in the same commit
(`cd benchmarks/argentum && cargo update -p topcoat -p toasty`) and keep its
revs identical to the root lockfile. Drift means the benchmark measures
different upstream code than the workspace builds.

## Commits

Every branch is squash-merged into `master`: one commit per branch, so no empty
merge commits. The squashed commit is a Conventional Commit with the issue
reference in the subject. [`docs/dev/COMMITS.md`](docs/dev/COMMITS.md) is the
authoritative format.

## Decisions and vocabulary

Record durable design decisions in [`docs/adr/`](docs/adr/). Domain terms and
the synonyms to avoid live in [`CONTEXT.md`](CONTEXT.md); use its words in code,
issues, and commits. All human-readable text follows
[`docs/dev/PROSE.md`](docs/dev/PROSE.md).
