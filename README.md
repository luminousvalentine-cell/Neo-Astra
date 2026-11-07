# Neo Astra

Neo Astra is a statically typed language that compiles to web, app, and system targets. Source files use the `.ns` extension, and you drive everything through the `astra` command-line tool.

This repo is the public source mirror. It holds the production source plus the files you need to get started. Everything else is covered in the docs linked below and in the language specification that ships with each release.

## Where things stand

Neo Astra `1.0.0` is out. The table below is the current support contract.

| Area | Status | Notes |
| --- | --- | --- |
| `core` mode | **Stable** | The only mode we call stable in 1.0. |
| `app` mode | **Partial** | The parts that exist work, but it isn't complete. |
| `web` mode | **Preview** | The capability and backend pieces are there, but there's no finished library surface yet. |
| `system` mode | **Preview** | Same story as web. |
| `high` mode | **Planned** | Not implemented in 1.0. |
| C17 target | **Shipped** | Emits executables, static libraries, and shared libraries. |
| JavaScript ESM target | **Shipped** | Produces readable ESM output. |
| TypeScript ESM target | **Shipped** | Produces readable TypeScript output. |
| Nova runtime | **Shipped** | Covers `run`, `eval`, `watch`, `repl`, and `test` during development. |
| NoPM | **Shipped** | Package graph, lockfile, store, and source adapters. |
| Language server (LSP) | **In progress** | Phase 1 is under construction and isn't in the 1.0 package. |
| Plugin host | **Later** | Not shipped. |
| Wasm execution backend | **Later** | Not shipped. |
| Browser playground | **Later** | Not shipped. |

The 1.0 release archives are built for **Windows x64**, **Linux x64**, and **macOS arm64**. Other platforms may build from source, but they aren't official release targets yet.

## Installing

The easiest route is npm. You'll need Node.js 22 or newer for the install step:

```sh
npm install --global neo-astra
astra --version
nopm --version
nova version
```

The npm package drops in native binaries for your machine, so once it's installed you don't need Node.js, npm, Cargo, or a source checkout to use Neo Astra. One thing to watch: don't pass `--ignore-scripts` when installing, or it won't work.

If you'd rather skip npm, grab a release archive from the GitHub releases page and unpack it:

```sh
tar -xf neo-astra-*.tar.gz
./neo-astra-*/bin/astra --version
```

[`INSTALL.md`](INSTALL.md) has the rest: prerequisites, archive layout, checksums, C toolchain setup, upgrading, and troubleshooting.

## Your first project

Make a folder to work in:

```sh
mkdir hello
cd hello
```

Add a `core.json`:

```json
{
  "package": { "name": "hello" }
}
```

Then a `build.json`:

```json
{
  "build": { "defaultTarget": "native", "defaultMode": "core" },
  "nova": { "entry": "src/main.ns" }
}
```

And finally `src/main.ns`:

```ns
module app.main //optional

pub fn main() -> i32 {
    println("Hello World")
    return 0
}
```

Now check it and build it:

```sh
astra check .
astra build . --target native
```

Native builds need a C compiler on your `PATH`, or a pinned toolchain declared in `build.json`. [`INSTALL.md`](INSTALL.md) explains how to set that up.

If you'd rather not type it all out, a ready-to-run copy lives in [`examples/hello`](examples/hello).

## What's in 1.0

- **A statically typed frontend.** That means a lexer, parser, resolver, capability and mode inference, a type checker, error effects, and structured diagnostics.
- **The `astra` CLI**, with `check`, `fmt`, `build`, `doctor`, `ast`, and `explain`.
- **Four backends:** native, C, JavaScript, and TypeScript. Inline `@script:js`, `@script:ts`, `@script:c`, and `@script:cpp` blocks are emitted with boundary validation. Any other script target is rejected with a clear error.
- **The `nova` development runtime.**
- **NoPM package management**, with support for path, Git, npm, Cargo, PyPI, NuGet, OCI, and `sys:` sources.
- **Standalone, stripped release archives** for the three supported platforms.

Still on the way: the language server, plugin host, Wasm execution, Web Worker code generation, the browser playground, and `high` mode.

## Contributing

Start with [`CONTRIBUTING.md`](CONTRIBUTING.md). The repo is owner-reviewed, so every change goes through a pull request, direct pushes to `master` are blocked, and published tags can't be changed. If your company uses Neo Astra commercially, we'd love for you to send back fixes, tests, or maintenance time. Apache-2.0 is permissive and doesn't require it but it makes a real difference.

## Security

If you find a vulnerability, please report it privately through the repository's security advisory flow, and don't open a public issue for something that isn't fixed yet. Details are in [`SECURITY.md`](SECURITY.md).

## License

Neo Astra is released under the Apache License 2.0. See [`LICENSE`](LICENSE), [`NOTICE`](NOTICE), and [`CONTRIBUTORS.md`](CONTRIBUTORS.md).

A note on comments: the public source mirror has them stripped from source files, and the compiler's generated output is comment-free too. If something needs explaining, it belongs in the documentation rather than in a source comment.