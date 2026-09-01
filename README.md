# ondas

Read-only Rust library for format-independent waveform analysis.

The public API is under development. This release contains the documented API
skeleton; waveform backends and query implementations are not available yet.

## Development

Linux development uses the public devcontainer. The host requires Git, Docker,
and the [Dev Container CLI](https://github.com/devcontainers/cli).

```sh
./dev just ci
./dev just msrv
```

The current gate checks formatting, Clippy, compilation, and API documentation.
It intentionally does not run tests while the library remains a skeleton.

Generate local API documentation with:

```sh
./dev just docs
```

Open `target/doc/ondas/index.html` after the command completes. Set
`ONDAS_FIXTURES` in a root `.env` to an absolute fixture-provider root when a
future command needs external waveforms; fixtures are never downloaded by
`./dev`.

## License

Licensed under Apache License 2.0.
