# Branching and releases

## Branches

| Branch | What it is | How changes get in |
|---|---|---|
| `main` | Development. Always builds and passes tests. The default branch. | Pull requests from `feature/*` and `fix/*` (small changes may be pushed directly) |
| `release` | Exactly what users get. Every merge is a release. | **Pull requests only**, from `main` (a normal release) or `hotfix/*` (an urgent fix). Enforced by a GitHub ruleset. |
| `feature/<topic>` | New work, e.g. `feature/windows-build` | Branch from `main`, PR into `main` |
| `fix/<topic>` | Bug fixes for the next release | Branch from `main`, PR into `main` |
| `hotfix/<topic>` | An urgent fix to the released version | Branch from `release`, PR into `release`, then merge `release` back into `main` |

```
feature/x ──PR──┐
fix/y ─────PR───┤
                ▼
main ──●────●────●────●────────●──────────►
                  \             ▲
           release PR           │ merge back
                    ▼           │
release ────────────●── v0.2.0 ─●── v0.2.1 ─►
                                ▲
             hotfix/z ──────PR──┘
```

## Rules on `release`

Enforced by the ruleset in [`.github/rulesets/release.json`](../.github/rulesets/release.json):
- **Changes only through a pull request.** No direct pushes, not even by admins (the ruleset has no bypass list).
- **No force-push, no deletion.**
- **All review conversations resolved** before merging.
- **CI must pass.** Added once the build workflow exists: the `required_status_checks` rule lists the CI job names.

`main` has a lighter ruleset ([`.github/rulesets/main.json`](../.github/rulesets/main.json)): no force-push and no deletion.

The ruleset requires **0 approvals**, because a project with one maintainer can't approve their own pull requests. The PR is still required, so every release is a reviewable, CI-checked unit with a record. Raise `required_approving_review_count` to 1 when there's a second maintainer.

## Making a release

1. On `main`, make sure CI is green and the release is ready.
2. Open a branch `release-prep/vX.Y.Z` from `main`: set `version` in `Cargo.toml` (`[workspace.package]`) and move the *Unreleased* notes in `CHANGELOG.md` under `vX.Y.Z`. PR it into `main` and merge.
3. Open a pull request **`main` → `release`** titled `Release vX.Y.Z`. The PR template has the release checklist. Merge when CI passes. Use a **merge commit** (not squash), so `release` and `main` share history.
4. Tag the merge commit on `release`: `git tag vX.Y.Z && git push origin vX.Y.Z`. The tag triggers the workflow that builds the installers and publishes the GitHub Release.

## Hotfixes

1. Branch `hotfix/<topic>` from `release`, fix, bump the patch version (`X.Y.Z+1`) and the changelog.
2. PR `hotfix/<topic>` → `release`, merge, tag `vX.Y.Z+1`.
3. Merge `release` back into `main` (via a PR) so the fix isn't lost.

## Versions

[Semantic versioning](https://semver.org): `MAJOR.MINOR.PATCH`. Before 1.0, a minor bump may change settings or behaviour. The version appears in the dock's footer and in the installer names.

## Setting up the protection (once)

**In the GitHub website:** repository **Settings → Rules → Rulesets → New ruleset → Import a ruleset**, and import `.github/rulesets/release.json`, then `main.json`. Branch protection on a free account is only available for **public** repositories.

**Or with the GitHub CLI:**

```sh
gh api repos/lastpatriot/obs-softphone/rulesets --method POST --input .github/rulesets/release.json
gh api repos/lastpatriot/obs-softphone/rulesets --method POST --input .github/rulesets/main.json
```

Create the `release` branch on GitHub before enabling its ruleset: `git push origin release`.
