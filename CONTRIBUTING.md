# Contributing to Neo Astra

Thanks for thinking about contributing! Neo Astra is owner-reviewed, so every change goes through a pull request and direct pushes to `master` are blocked. Published tags and release assets can't be changed once they're out.

## Before you start

- Take a look at the status table in [`README.md`](README.md). While `app` is partial and `web` and `system` are previews, please don't describe `app` as complete or the other two as stable.
- Keep your changes focused. A pull request that changes one behavior is much easier to review, and to revert if it comes to that.
- Please don't add generated output, build products, secrets or private test files.

## License and sign-off

Neo Astra is licensed under Apache-2.0. When you submit a pull request, you're agreeing that your contribution is licensed under Apache-2.0 and that you have the right to submit it.

Sign off on every commit:

```sh
git commit -s -m "describe the change"
```

The sign-off certifies the Developer Certificate of Origin. You don't need to sign a separate CLA.

## How the public mirror works

This repository mirrors the production source. A few things follow from that. The mirror:

- strips comments from source files, including `.ns` examples;
- leaves out the private test suite, the conformance corpus, and internal planning docs;
- is append-only after a published tag.

Because of this, explain behavior in the documentation or in tests, not in source comments. A comment isn't part of the public contract.

## Pull request checklist

- [ ] The change is scoped to one problem.
- [ ] Tests cover the success path and every failure mode the change could hit.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes.
- [ ] The relevant test suite passes.
- [ ] Any public claims match the real support matrix.
- [ ] No published tag or release archive was rewritten.

Before landing a change, maintainers run the private gate. We may accept a pull request without tests if it's documentation-only, but not if it touches compiler behavior.

## For commercial users

Apache-2.0 lets companies use, modify, and distribute Neo Astra with no obligation to give anything back. We'd still love it if you did. Fixes, tests, documentation and maintenance time all help. If your company needs a support or maintenance agreement, open a discussion with the repository owner. The license doesn't include one, so please don't assume it does.

## Code of conduct

Be direct about the code and kind to the people. We don't accept harassment, personal attacks, or the disclosure of private reports.