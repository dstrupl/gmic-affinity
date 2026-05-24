<!-- vardoger:start -->
# Personalization

## Communication
- Default to concise responses, but expand structure and detail when the task itself includes explicit formatting requirements.
- Use Markdown headings and clearly labeled sections when the user asks for structured output.
- Expect a mix of casual check-ins and direct task requests; keep casual replies light and task responses focused. (tentative)

## Workflow
- When the user requests written documentation, keep it short, direct, and specific to the repository or task.
- Adapt templates to the actual context instead of forcing every suggested section or pattern.

## Coding Style
- Include concrete examples such as commands, paths, and naming patterns when they make the output more actionable.

## Things to Avoid
- Avoid padding deliverables with unnecessary length or irrelevant sections.

<!-- vardoger:end -->

# Repository Instructions

## Project Context
- `gmic-affinity` is a Rust 2021 macOS Photoshop-compatible `.plugin` bundle for Affinity Photo 2 and Affinity Photo v3.
- The crate builds as `staticlib`/`rlib`; the Makefile relinks the static archive with `clang -bundle` so the bundle executable is `MH_BUNDLE` and exports `_PluginMain`.
- The default build is a no-op `PluginMain`. Use `FEATURES=live` for the real G'MIC pixel pipeline and Cocoa picker.
- The bundled G'MIC catalogue at `assets/gmic-catalogue.gmic.gz` is Git LFS-tracked. If builds fail at `check-lfs`, run `git lfs pull`.

## Key Files
- `README.md`: install, build, troubleshooting, release overview.
- `SETUP.md`: one-time local macOS development setup.
- `IMPLEMENTATION_NOTES.md`: architecture, Photoshop SDK details, release runbooks.
- `PRD.md`: product requirements and status.
- `Makefile`: source of truth for build, install, test, release, and verification targets.
- `src/`: plugin implementation, G'MIC bridge, TIFF IO, settings, catalogue parser, and AppKit picker.
- `tests/`: layout, catalogue snapshot, and error matrix tests.

## Build And Test Commands
- Format: `cargo fmt --all -- --check`
- Lint: `cargo clippy --all-targets --all-features -- -D warnings`
- Tests: `make test`
- Live-only tests: `cargo test --locked --features live`
- Quality guardrails: `make quality-metrics`
- Fast bundle: `make bundle`
- Live universal bundle: `make universal FEATURES=live`
- Install into detected Affinity plugin folders: `make install`
- Standalone picker: `make picker-example`

## Development Guidelines
- Prefer existing Makefile targets over hand-written command sequences; they encode bundle verification, signing, resource copying, and LFS checks.
- Keep the no-op default build and `live` feature split intact unless the task explicitly changes release/build strategy.
- Do not replace the legacy `GmicFilter.rsrc` or modern `PiPLs.json` paths casually; Affinity depends on the legacy resource for plugin discovery.
- Preserve the `staticlib` plus `clang -bundle` pipeline unless you are specifically working on bundle-loading behavior.
- Treat `FilterRecord` layout changes as high-risk. Update layout tests and cross-check against the Photoshop SDK notes before touching `src/ps_types.rs`.
- Keep release-signing and notarisation behavior aligned with `release/notarisation/SIGNING.md` and `IMPLEMENTATION_NOTES.md`.
- Avoid broad refactors around the AppKit picker and Objective-C bridge unless needed for the requested behavior; small memory-management mistakes can crash the host process.

## Verification Expectations
- For ordinary Rust logic changes, run the narrow relevant tests plus `cargo fmt --all -- --check`.
- For changes touching shared plugin behavior, picker code, G'MIC invocation, TIFF IO, settings, or catalogue parsing, run `make test` when feasible.
- For build, bundle, resource, signing, or release changes, run the relevant Makefile target (`make bundle`, `make universal FEATURES=live`, or release preflight target) and inspect the failure mode before editing further.
- If a command cannot run because the host lacks macOS tools, Affinity, G'MIC, Photoshop SDK, or hydrated LFS assets, state that explicitly and give the exact command that failed.

## Release Notes
- Release source of truth: `docs/design/2026-05-18-release-v0.1-distribution.md` is the design record; `IMPLEMENTATION_NOTES.md` section 11 is the operator runbook; `release/notarisation/SIGNING.md` is the signing collaborator guide.
- There are two release pipelines:
  - Pre-release / RC tags are hyphenated (`vX.Y.Z-rc.N`, `vX.Y.Z-beta.N`). They trigger `.github/workflows/release.yml`, which builds `make release-unsigned` as a universal `FEATURES=live` ad-hoc-signed zip and publishes it as a GitHub prerelease.
  - Stable tags are bare semver (`vX.Y.Z`). They do not go through CI release publishing. They go through the collaborator-run `make release RELEASE_VERSION=vX.Y.Z` pipeline with Developer ID signing, notarisation, stapling, GitHub release publishing, and Homebrew tap cask bump.
- Do not invent release versions. For stable releases, the project lead chooses `vX.Y.Z`; `Cargo.toml` `package.version` and `Info.plist` `CFBundleShortVersionString` must already match `X.Y.Z` before handoff.
- Stable release preflight intentionally refuses dirty trees, detached HEADs, branches not in sync with origin, invalid notary credentials, missing Developer ID identity, unreachable tap repo, existing conflicting releases/tags, and missing release inputs. Fix the cause rather than bypassing `release-preflight`.
- Signing credentials stay local to the collaborator's Mac. Never commit `.env.local`, Apple IDs, app-specific passwords, certificate material, notarytool profiles, or any value copied from a keychain.
- The expected successful stable verification includes `spctl --assess` reporting `source=Notarized Developer ID`. If Gatekeeper reports anything else, stop and do not publish or continue the release.
- The Homebrew tap repo is `dstrupl/homebrew-gmic-affinity`. The staged files under `release/homebrew-tap/` document and bootstrap the tap; the live cask should be changed in the tap repo directly or via `scripts/release-bump-cask.sh`, not by treating the staged copy as an automatically-published source.
- The cask path depends on a notarised bundle because Affinity rejects quarantined ad-hoc-signed plugins and Homebrew removed the old quarantine workaround. The hard Homebrew policy deadline documented here is 2026-09-01 for non-Gatekeeper-passing casks.
- If `release-bump-cask` fails after the GitHub release is published, re-run `scripts/release-bump-cask.sh` with the same version, zip path, and tap URL; it is designed to be idempotent.
- Rollback for a broken stable release is: delete the GitHub release, revert the tap cask bump, then record the failure mode in the release design notes. Pre-release rollback usually only needs deleting the prerelease.
