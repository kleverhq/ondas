# ondas

Read-only Rust library for format-independent waveform analysis.

Reads FST and VCD files and in-memory bytes through independent `fst-native` and
`vcd-native` backends. FST uses unmodified `fst-reader`; VCD is parsed directly
without a history database. Other waveform formats have no reader in this release.
Requires Rust 1.88 or newer.

## Public Documentation

The [API documentation on docs.rs](https://docs.rs/ondas) explains the public
contracts and usage. Its source is rustdoc in `src/`.

## Development

Linux development uses the public devcontainer. The host requires Git, Docker,
and the [Dev Container CLI](https://github.com/devcontainers/cli).

```sh
./dev --install-hooks
./dev just check-local
# Configure ONDAS_FIXTURES in .env before installation/full CI.
./dev just fixtures-install
./dev just ci
./dev just msrv
```

The CI recipe runs `check-local` (formatting, Clippy, compilation, API documentation,
self-contained Rust tests/doctests, and repository-tool tests) and mandatory
conformance. Pre-commit runs only `check-local`. Conformance requires the provider pinned in `fixtures.lock.toml`
and supplied through `ONDAS_FIXTURES` in the ignored root `.env`. Missing fixtures
fail that suite rather than being skipped. `fixtures-install` clones the locked
GitHub tag and runs its checksum-verifying installer; CI never installs implicitly. It discovers every FST and VCD in the provider
and checks its listed oracle observations in file and bytes modes, alongside
focused API regressions. Per-case results remain visible even when another case
fails; sparse observations are not exhaustive coverage of each artifact.

See the public API documentation for reader limitations: some malformed FSTs can
trigger upstream panics, and first-tick event callbacks can include initialization.
Character bytes are mapped as Latin-1 rather than guessed as UTF-8. VCD checkpoint
event records are not counted as occurrences; resume records do not reveal the
physical time of hidden changes.

Hook installation is explicit and per worktree. Start that worktree's container
before committing on the host: the pre-commit hook runs the gate on staged changes
through the existing container without starting or rebuilding it. Reinstall hooks
after reviewing changes to `dev` or `tools/repo/git-hook`. Use
`./dev just pre-commit` to check all tracked files manually.

Generate local API documentation with `./dev just docs`, then open
`target/doc/ondas/index.html`.

Developer sources of truth:

- [Library model](docs/model.md)
- [Testing](docs/testing.md) and [fixture catalog/oracles](docs/fixtures.md)
- [Benchmarking](docs/benchmarking.md)
- [Environment and automation](docs/automation.md)
- Format integration: [VCD](docs/vcd.md), [FST](docs/fst.md),
  [GHW](docs/ghw.md), [FSDB](docs/fsdb.md), [WLF](docs/wlf.md)

Agent guidance starts in `AGENTS.md`. Use ignored `tmp/` for local scratch and
`docs/wip/yymmdd-slug/` for temporary work that needs committing; remove task
directories before merge into `master`.

## License

Licensed under Apache License 2.0.
