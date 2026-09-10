# Developer documentation

## Sources of truth

- Public API contracts and examples belong in rustdoc under `../src/`, not in a Markdown API reference.
- `model.md` owns the conceptual model; `testing.md` owns test strategy; `fixtures.md` and `oracle.schema.json` own the fixture contract; `benchmarking.md` owns measurement policy; `automation.md` owns maintainer workflow.
- Backend internals belong in their own documents, starting with `vcd-native.md`. Other integration constraints remain in `fst.md`, `ghw.md`, `fsdb.md` and `wlf.md`. Keep user-visible reader selection and limitations in Rustdoc.
- Available commands, dependencies, and toolchain versions come from `../justfile`, `../Cargo.toml`, `../rust-toolchain.toml`, and `../.devcontainer/`.

## Writing

- Describe concepts and constraints directly. Keep implementation status, changelogs and execution plans out of normative docs.
- Link to the owner of a contract instead of maintaining parallel copies. Keep schema structure in `oracle.schema.json` and its semantic constraints in `fixtures.md`.
- Keep tracked temporary work only in `wip/yymmdd-slug/`; remove those task directories before merge into `main`.
