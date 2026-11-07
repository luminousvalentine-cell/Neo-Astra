# Examples

Each directory is a standalone Neo Astra project. Run commands from the example
directory so the project-local manifests and caches stay together.

## hello

```sh
cd hello
astra check .
astra build . --target native
```

`check` validates the project manifests and runs the frontend. `build` needs a
C compiler on `PATH`, or a pinned toolchain declared in `build.json`.

Set `build.defaultTarget` to `js` or `ts` to try the source backends instead.

All public `.ns` examples are comment-free because the public source mirror
strips comments. Put explanation in the example README instead.
