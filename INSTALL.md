# Install and upgrade

Neo Astra `1.0.0` ships as a host-native package. Once it's installed, you don't need Node.js, npm, Cargo, or a source checkout to run it. There's no background service and nothing is stored globally beyond the installed commands.

## Supported hosts

| Host | Release archive | npm platform package |
| --- | --- | --- |
| Windows x64 | yes | `neo-astra-windows-x64` |
| Linux x64 | yes | `neo-astra-linux-x64` |
| macOS arm64 | yes | `neo-astra-darwin-arm64` |

Other hosts can build from source, but they aren't covered by the 1.0 release contract.

## Install from npm

You'll need Node.js 22 or newer, but only for the install step:

```sh
npm install --global neo-astra
astra --version
nopm --version
nova version
```

Don't use `--ignore-scripts`, because the install script is what puts the host-native binaries in place. Think of the npm package as an installer rather than a runtime: after it finishes, `astra`, `nopm`, and `nova` run without Node.js or npm.

## Install from a release archive

Download the archive for your host from the GitHub releases page, along with its matching `.sha256` sidecar file.

```sh
sha256sum -c neo-astra-*.sha256
tar -xf neo-astra-*.tar.gz
./neo-astra-*/bin/astra --version
```

On Windows, check the hash with `Get-FileHash -Algorithm SHA256` and extract the `.zip` with `Expand-Archive`.

Here's what's inside the archive:

- `bin/astra` (`.exe` on Windows) is the CLI, including `astra nopm`.
- `bin/nopm` (`.exe` on Windows) is a launcher for `astra nopm`.
- `bin/astra-compiler` is the native compiler.
- `bin/nova` is the Nova development runtime.
- `typescript/tsc` is the pinned native TypeScript compiler that `@script:ts` uses.

The binaries are compiled and stripped, so the package has no TypeScript sources, source maps, comments, or debug symbols.

A `.sha256` sidecar catches corrupted downloads, but it can't prove an archive wasn't swapped out. For releases after `1.0.0` we turn on provenance attestations. When one is present, you can verify it with the GitHub CLI:

```sh
gh attestation verify neo-astra-*.tar.gz --repo luminousvalentine-cell/Neo-Astra
```

## Requirements

- **Node.js 22+ with npm** is needed for the npm install step and for building from source. An installed package doesn't need it at runtime.
- **Rust stable** is needed for source builds. If it's missing, the bootstrap installs it for you through `rustup`.
- **A C compiler** is needed for `native` and `c` builds. That means `clang`, `gcc`, or `cc`. On Windows you'll also need a hosted linker (MSVC build tools or a complete MinGW toolchain) to produce executables.
- **Git** is needed for Git package sources. NoPM also calls on npm, Cargo, and system package managers, but only when a project asks for a source that depends on them.

## First run

```sh
astra --help
astra check path/to/project
astra fmt path/to/project --check
astra build path/to/project --target native
astra doctor
```

`astra check` validates the project manifests and runs the frontend. `astra build` picks its target from `build.json` or `build.toml`, or from `--target`, which accepts `native`, `c`, `js`, or `ts`. `astra doctor` shows how the C toolchain gets resolved.

A minimal project needs exactly one core manifest (`core.json` or `core.toml`) and, optionally, a build manifest (`build.json` or `build.toml`). [`examples/hello`](examples/hello) shows what that looks like.

## C toolchain

`astra doctor` looks for a C compiler in this order:

1. `ASTRA_CC`, if you've set it.
2. A pinned local cache under `<project>/.tools/toolchain/<id>/`.
3. `clang`, `gcc`, or `cc` on your `PATH`, preferring one that passes the hosted probe.

You set the policy in your build manifest:

```json
{
  "toolchain": { "c": "auto" }
}
```

`auto` picks a hosted compiler from `PATH`. A bare family pin like `clang` or `gcc` uses the cache when it has an entry and may fall back to `PATH`. A versioned pin such as `zig@0.13.0` or `clang@18` never trusts `PATH`.

With a versioned pin, Astra downloads the official archive for your OS and architecture, checks its byte size and SHA-256, unpacks it with the system `tar`, and installs it under the project. Pass `--no-download` to skip the fetch. If anything goes wrong, the error names the archive and tells you how to install it manually.

## Upgrading

With npm:

```sh
npm update --global neo-astra
astra --version
```

With an archive, download the new one, verify its sidecar, and replace the old unpacked directory. Published tags are immutable, so never edit a released archive in place. Any source change after `1.0.0` goes out as a new patch or minor version.

All package versions move together. If `astra --version` doesn't match the package metadata, reinstall rather than adjusting one version by hand.

## Uninstalling

With npm:

```sh
npm uninstall --global neo-astra
```

With an archive, just delete the unpacked directory. Each project keeps its own state in `.astra/` and `.tools/`. Only remove those when you no longer need the project's store or toolchain cache.

## Troubleshooting

| Symptom | What it means |
| --- | --- |
| `NS9001` | The project has no core manifest, or it has both `core.json` and `core.toml`. You need exactly one. |
| `NS9003` | The `astra-compiler` binary couldn't be found. Reinstall the package. |
| `NS9101` | The `toolchain.c` policy is malformed. |
| `NS9102` | A pinned toolchain cache entry is missing, mismatched, or incomplete. |
| `NS9103` | `ASTRA_CC` points to something that isn't a usable C compiler. |
| `NS4019` | A task handle would cross a task boundary as a copied value. |

Remember that `app` mode is partial, and `web` and `system` are previews in 1.0. If a mode or standard-library feature isn't there, the compiler should reject it explicitly. If you hit a silent fallback or a build that succeeds when it shouldn't, please report it as a bug.