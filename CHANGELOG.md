# Changelog

Every notable change to Neo Astra is recorded here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project sticks to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- The Apache-2.0 license, a `NOTICE` file, and `CONTRIBUTORS.md`.
- The public onboarding files: `README.md`, `INSTALL.md`, `SECURITY.md`, `CONTRIBUTING.md`, `.github/CODEOWNERS`, and `examples/hello`.
- npm provenance and GitHub release artifact attestations.

### Changed

- The public source mirror is now append-only after the `v1.0.0` tag. Published tags and release assets can't be changed.
- The source mirror strips comments from public source, including the comments in `.ns` example files.

### Work in progress

- Native language server, phase 1: a stdio LSP with diagnostics and formatting.

## [1.1.2] - 2026-11-28

### Added

- Edition `0.2` `if let` / `while let` desugaring, ranges (`a..b`, `a..=b`) with `for` iteration and slicing, integer bitwise operators, `char` and single-scalar literals, immediate lambdas, and the complete integer/float suffix table. Edition `0.1` is unchanged.
- Array and slice `slice(start, end)` support across the checker and all three backends.

### Fixed

- Matching a value inside `println` now infers the match result from its arms instead of inheriting the builtin's opaque parameter type.

## [1.1.1] - 2026-11-26

### Fixed

- The packaging smoke fixture uses a supported edition so the packaged `astra check` succeeds.

## [1.1.0] - 2026-11-24

### Added

- Language edition `0.2` syntax cleanup: `fail Type(...)`, postfix `?`, `::<...>` turbofish, canonical scope blocks, canonical aliases, and a deny-by-default canonical-syntax lint.
- Source-to-source migration core for edition `0.2`.

### Changed

- The formatter is edition-aware and emits canonical `0.2` spellings while preserving `0.1` spellings under edition `0.1`.

## [1.0.1] - 2026-11-22

### Added

- Native Rust `astra` and `nopm` binaries replace the Bun-compiled CLI and the `nopm` launcher. Baseline `check`, `fmt`, `ast`, `explain`, `doctor`, C and native builds, and NoPM path flows run without Node.js, npm, Cargo, or a source checkout.
- JS and TypeScript code generation is delegated to an isolated, on-demand Node bridge. JS/TS builds are the only path that needs Node.js at runtime.
- Native toolchain resolution and verified pinned downloads with `--no-download`, plus a native NoPM store, lockfile, and registry adapters.
- Release archives now include the native compiler and Nova binaries, the pinned TypeScript compiler, and smoke coverage for packages installed outside the repo.

### Fixed

- Packaged JS/TS builds now run when the package lives under a symlinked path, including macOS temporary directories.
- Native `--version` output and generated lockfiles now follow the workspace version instead of a hardcoded `1.0.0`.

## [1.0.0] - 2026-11-06

### Added

- The v0.1 language frontend, made up of a lexer, parser, AST, module resolver, capability and mode inference, type checker, error effects, and structured diagnostics.
- Four modes, `core`, `app`, `web`, and `system`, with least-privileged inference. `core` is stable, `app` is partial, and `web` and `system` are previews.
- C17, JavaScript ESM, and TypeScript ESM backends, including inline `@script:js`, `@script:ts`, `@script:c`, and `@script:cpp` blocks.
- The `astra` CLI, with `check`, `fmt`, `build`, `doctor`, `ast`, and `explain`.
- The `nova` development runtime, with `run`, `eval`, `watch`, `repl`, `test`, and `version`.
- NoPM package management, with path, Git, npm, Cargo, PyPI, NuGet, OCI, and `sys:` sources.
- Pinned Zig 0.13.0 C toolchain downloads, with size and SHA-256 verification, a project-local cache, and guidance when `--no-download` causes a failure.
- Standalone release archives for Windows x64, Linux x64, and macOS arm64, plus the `neo-astra` npm meta package and three platform packages.
- Structured `TaskError` results at `join`, `task.canceled()`, rejection when a task handle would be copied across a task boundary (`NS4019`), and the system-mode evaluation-order rule (`NS5007`).
- Project-local `.astra/` and `.tools/` state. Nothing needs elevated permissions, and nothing is written outside the project.

### Known limitations

- `app` mode is partial.
- `web` and `system` modes are previews.
- `high` mode is planned but not implemented.
- The language server, plugin host, Wasm execution backend, Web Worker code generation, and browser playground aren't part of 1.0.
- Inline scripts in languages other than JS, TS, C, and C++ are rejected outright instead of being emulated.