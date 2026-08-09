# Contributing to AutoForge

Thanks for helping improve AutoForge. Bug reports, documentation fixes and
focused pull requests are welcome. Participation is governed by the
[Code of Conduct](CODE_OF_CONDUCT.md).

## Before you start

- Search existing issues and pull requests before opening a new one.
- For a large feature or architecture change, open a proposal issue first so
  scope and direction can be agreed before implementation.
- Report vulnerabilities through GitHub's private security advisory flow as
  described in [SECURITY.md](SECURITY.md), never in a public issue.

## Development setup

AutoForge requires Node.js 18 or newer, Rust 1.88 or newer, and the
[Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/) for your
operating system.

```bash
git clone https://github.com/YOUR-USER/AutoForge.git
cd AutoForge
git remote add upstream https://github.com/vima-tech/AutoForge.git
git fetch upstream
git switch -c feat/my-change upstream/dev
npm ci
npm run tauri:dev
```

The integration branch is `dev`. Please target `dev` when opening a pull
request. Maintainers promote tested releases from `dev` to `main`.

## Validation

Run the checks relevant to your change before opening a pull request:

```bash
npm run version:check
npm run build
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets
cargo test --locked --manifest-path src-tauri/Cargo.toml
```

For UI or IPC changes, also run `npm run tauri:dev` and manually exercise the
affected flow. Browser-only Vite mode cannot validate Tauri APIs.

## Pull requests

- Keep each pull request focused on one coherent change.
- Explain the problem, the approach, user impact and validation performed.
- Add or update tests for behavior changes.
- Update documentation when commands, configuration or user-facing behavior
  changes.
- Do not commit credentials, local databases, generated build output or
  machine-specific files.

By submitting a contribution, you agree that it is licensed under the
[Apache License 2.0](LICENSE), as described in section 5 of that license.
