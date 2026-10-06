# Developer documentation

## Sources of truth

- Public API contracts and examples belong in rustdoc under `../src/`, not in a Markdown API reference.
- `model.md` owns the conceptual model; `testing.md` owns test strategy; `fixtures.md` owns fixture integration and oracle interpretation; `benchmarking.md` owns measurement policy; `automation.md` owns maintainer workflow. The locked fixture provider owns catalog, sidecar and oracle schemas.
- Backend internals belong in their own documents, including `vcd-native.md`, `fst-lib.md` and `fsdb-lib.md`. Other integration constraints remain in `ghw.md` and `wlf.md`. Keep user-visible reader selection and limitations in Rustdoc.
- Available commands, dependencies, and toolchain versions come from `../justfile`, `../Cargo.toml`, `../rust-toolchain.toml`, and `../.devcontainer/`.

## Writing

- Describe concepts and constraints directly. Keep implementation status, changelogs and execution plans out of normative docs.
- Link to the owner of a contract instead of maintaining parallel copies. Link fixture schemas to the provider release selected by `../fixtures.lock.toml`; keep Ondas runner coverage and evidence interpretation in `fixtures.md`.
- Keep tracked temporary work only in `wip/yymmdd-slug/`; remove those task directories before merge into `main`.
