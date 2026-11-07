# Security policy

## Supported versions

We ship security fixes for the current `1.0.x` release line. If you're on an older release, we may ask you to upgrade before we consider backporting a fix.

## Reporting a vulnerability

Please don't open a public issue for a vulnerability that hasn't been fixed yet.

Instead, use the repository's private security advisory flow:

1. Open the **Security** tab on the public repository.
2. Choose **Report a vulnerability**.
3. Tell us the affected version and host, give us a minimal reproduction and describe the impact.

If the advisory flow isn't available for some reason, reach out to the repository owner through the GitHub profile linked from the repository. Please do that before disclosing anything publicly.

## What to expect

- We'll acknowledge your report.
- We'll confirm it, or tell you why we're rejecting it.
- If we accept it, we'll work with you on a coordinated fix and release.
- We'll credit you in the release notes, unless you'd rather stay anonymous.

## Scope notes

- The compiler runs on untrusted source code, so a panic, hang, or parser crash on malformed input is a bug and we'd like to hear about it.
- Release archives come with SHA-256 sidecar files. If a checksum doesn't match, the download is most likely corrupt. On its own, a mismatch doesn't prove someone swapped the file with something malicious.
- A missing feature isn't a security issue. The exception is a feature that's advertised as working but quietly falls back to unsafe behavior.