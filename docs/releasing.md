# Releasing Hokum

One run of the **Release** workflow (`.github/workflows/release.yml`) builds
and publishes Hokum for Linux x86_64 and aarch64 from one commit to the
Anomaly release service (`anomalyco/release`, `https://anoma.ly/release`).
It follows Hex's release (anomalyco/hex `docs/releasing.md`), without the
macOS and Windows signing, Homebrew, Nix, or site jobs. The service's dev
stage, `https://dev.anoma.ly/release`, is only for testing the release service
itself: Hokum never publishes to it. Builds are tested before stable on the
production service's `dev` channel.

Like Hex's and OpenCode's releases, building is split from everything that
holds a secret. `.github/workflows/release.yml` and `.github/release/` are
the security boundary:

- **Build jobs** run repository code (cargo, the build scripts, nfpm,
  makepkg) with no environment, no secret, and no id-token, and make unsigned
  files.
- **Publish jobs** run in the `release` environment (the tag job instead has
  `contents: write`), check out only `.github/release/` (sparse checkout),
  and run only those scripts and SHA-pinned actions, on artifacts they treat
  as data. They never build, install dependencies, or run repository code or
  the artifacts.
- **Artifacts** each carry a `SHA256SUMS`; the job that made one reports that
  manifest's digest as a job output, and every job that downloads it checks
  the digest and every file (`.github/release/verify-artifacts.sh`), so
  nothing swapped in between jobs is published.

```ts
Release (push to dev: dev channel; workflow_dispatch: ref + channel check | dev | stable (from production))
├─ plan                .github/release/plan.sh: version, channel, tag, notes
├─ linux-x64           scripts/build-linux.sh + package-linux.sh on ubuntu-24.04: the executable,   [build]
│                      .deb and .rpm (installed and run on Ubuntu and Fedora), a headless render
├─ linux-arm64         the same on ubuntu-24.04-arm                                                 [build]
├─ publish             OIDC: every target, then the release (release-service.sh)                     [release]
├─ github              last: tag v<version> on the released commit (tag-release.sh), then the        [contents: write]
│                      GitHub release mirror (publish-github-release.sh); GITHUB_TOKEN only.
│                      check: pushes and deletes a throwaway v0.0.0-test-<run> tag
├─ aur-stage → aur     stable: makepkg + install + render from the published files → push-aur.sh    [build → release]
└─ record-aur          OIDC: the AUR serves the version                                              [release]
```

### Release tags

`v*` tags are not restricted: tags trigger nothing. The final `github` job
pushes `v<version>` with the built-in `GITHUB_TOKEN` (`contents: write` on
that job only, no environment).

### Who can change the boundary

`.github/CODEOWNERS` names @thdxr for `/.github/`. GitHub has no push rulesets
(file-path restrictions) for public repositories, so the lock is the branch
ruleset "release boundary (.github needs Dax)" on `dev` and `production`:
pull requests only, with code-owner review, and Dax as the one bypass actor.
Repository admins can still change rulesets.

## The release service

The service authorizes the publish jobs by their GitHub OIDC token alone:
`registry/hokum.json` in `anomalyco/release` admits repository
`anomalyco/hokum` (by name and id) and workflow `.github/workflows/release.yml`
running from the same ref it was triggered on, for artifact `desktop`, as two
publishers:

```ts
release  refs/heads/production                   channels stable, beta
release  refs/heads/dev                            channel  dev
```

Every run uses one environment, `release`, with no required reviewer (every
push to `dev` publishes); its deployment policy admits `dev` and
`production`. The registry ties channels to refs, so a build of `dev` can
never publish to `stable`. No key or secret publishes: only the `publish` and
`record-aur` jobs present an OIDC token.

`publish` uploads each file under its own name with its SHA-256, then posts
the release naming each target by file and hash, with distributions `direct`,
`aur` (`hokum-bin`), `deb-x64`, `deb-arm64`, `rpm-x64`, and `rpm-arm64`. It
then downloads every published URL and compares it with the local bytes.
Package-manager entries gain a version only once that package manager serves
it: the `deb`/`rpm` kinds at publish (the packages are files of the
release) and `aur` after the push. Manifests are not signed; Linux has only
HTTPS and the SHA-256.

### Names

```ts
https://anoma.ly/release/hokum/            the service
├─ index.json
└─ desktop/<channel>.json                  channels stable, beta, dev
   └─ release.targets
      ├─ linux-x64            hokum         bare executable (AUR)
      ├─ linux-x64-deb        hokum.deb     Debian/Ubuntu package
      ├─ linux-x64-rpm        hokum.rpm     Fedora/openSUSE package
      ├─ linux-arm64          hokum
      ├─ linux-arm64-deb      hokum.deb
      └─ linux-arm64-rpm      hokum.rpm
```

Published names carry no version or target: each file lives at
`/release/hokum/files/<sha256>/<name>`, which tells them apart. The builds
and the GitHub release mirror keep versioned names
(`hokum-<v>-linux-x64.deb`, …); `.github/release/release-service.sh` maps
them. Every target has a stable link to the channel's current file, for
example `https://anoma.ly/release/hokum/desktop/dev/linux-x64-deb`.

Hokum has no updater: installs update through their package manager, or by
installing the newest file again. The running service (`src/update.rs`) does
check in, at startup and then hourly (plus up to five minutes of jitter): it
fetches its channel's manifest with User-Agent `hokum/<channel>/<version>`
(for example `hokum/stable/0.1.1` or `hokum/dev/0.0.0-dev-19`), which counts
active installs, and logs when the manifest's version is newer. The channel
comes from the built version: `0.0.0-dev-*` is dev, `*-beta.*` beta,
anything else stable. Failures are silent, and `check-for-updates #false` in
config.kdl's `behaviour` turns it off.

## Channels

- `check` (the dispatch default, run from `dev`) builds, packages, and tests
  everything as `0.0.0-dev-<run number>` and publishes nothing anywhere: a
  dry run of the build and packaging.
- `dev` publishes a build of `dev` to the production service's `dev` channel
  (`https://anoma.ly/release/hokum/desktop/dev.json`). Every push to `dev`
  runs it, except pushes that change only `docs/` or Markdown; a running dev
  release always finishes, and only the newest push that lands meanwhile
  waits to run next (GitHub drops the ones between), and
  `gh workflow run release.yml --ref dev -f channel=dev -f ref=dev` runs it
  by hand. The version is `0.0.0-dev-<run number>`, as OpenCode and Hex
  number their dev builds (`scripts/set-crate-version.sh` stamps it into the
  build, never committed). Versions are display text that nothing compares:
  each channel has one current release. It publishes only the release: no
  GitHub release, tag, or AUR.
- `stable`, dispatched from `production` (`gh workflow run release.yml
  --ref production -f channel=stable -f ref=production`), publishes the crate
  version from `Cargo.toml` to `https://anoma.ly/release` and the AUR. It
  needs `docs/releases/<version>.md` and refuses a version already tagged
  `v<version>`. Tags trigger nothing. Only once the release service
  published does the `github` job tag `v<version>` on the released commit
  with `GITHUB_TOKEN` and create the GitHub release `v<version>` ("Latest")
  with the same files. A failed run leaves no tag.

To ramp, halt, roll back, or force an update, use the service's actions
(`rollout`, `halt`, `promote` with a history `sequence`, `required`;
`anomalyco/release` docs/publishing.md) from the admin panel or a job with
the same OIDC identity.

## Cutting a release

Work lands on `dev`. `production` holds what was released and moves only by a
fast-forward from `dev`; nobody commits to it directly, and its ruleset
refuses force pushes and deletion.

1. Bump `version` in `Cargo.toml` (X.Y.Z), refresh `Cargo.lock`
   (`cargo check`), add `docs/releases/<version>.md`, push to `dev`.
2. Let the push to `dev` publish its dev-channel build and try it
   (`https://anoma.ly/release/hokum/desktop/dev.json`).
3. Fast-forward `production` to that commit (`git push origin
   <commit>:production`, which refuses anything but a fast-forward; the
   "release boundary" ruleset lets only Dax push it directly, others open a
   pull request), then run Release from `production` with `channel: stable`
   (its default `ref`). Stable runs refuse any other branch. The run tags
   `v<version>` itself at the end; never push a release tag by hand.
4. Check `https://anoma.ly/release/hokum/desktop/stable.json`,
   `https://github.com/anomalyco/hokum/releases/tag/v<version>`, and
   `https://aur.archlinux.org/packages/hokum-bin`.

If a job fails after the release service published, fix the cause and rerun
the failed jobs of the same run: publishing the channel's current version
again succeeds only when the service already has exactly these files, and the
tag, GitHub release, and AUR steps accept what they already published.

## Linux packages

- **AUR** `hokum-bin` (`packaging/aur`, staged by `scripts/release-aur.sh`,
  pushed by `.github/release/push-aur.sh`): `/usr/bin/hokum` from the release
  service's `linux-x64` (`x86_64`) or `linux-arm64` (`aarch64`) file, the
  systemd user unit `/usr/lib/systemd/user/hokum.service`, the D-Bus
  activation file `/usr/share/dbus-1/services/app.hokum.Notifications.service`
  (both run `hokum service`), the launcher entry
  `/usr/share/applications/hokum.desktop` (plain `hokum`, the settings
  window), and the license. Maintained under Dax's AUR account.
- **.deb and .rpm** `hokum` (`packaging/linux/nfpm.yaml`, built by
  `scripts/package-linux.sh` with nfpm in the build jobs): the same files as
  the AUR package, for x64 and arm64, published as `linux-<arch>-deb` and
  `linux-<arch>-rpm` (`hokum.deb`, `hokum.rpm`). There is no apt or dnf
  repository: the release service's `deb-<arch>`/`rpm-<arch>` distributions
  carry the command that downloads the stable package from its link and
  installs it with `apt` or `dnf`. Package versions turn the first hyphen into
  a tilde (`0.0.0~dev.42`, `0.2.0~beta.1`).
- **Source**: `cargo build --release` and the files in `packaging/`.

The packages provide `notification-daemon` (deb; `hokum-bin` too, as dunst
and xfce4-notifyd do) or `desktop-notification-daemon` (rpm). Like mako,
dunst, and swaync, the D-Bus activation file has the daemon's own name rather
than `org.freedesktop.Notifications.service`, so it shares no file with any
other daemon. The running daemon owns the bus name; `hokum.service` has
`Conflicts=mako.service dunst.service swaync.service`.

Both architectures build natively on Ubuntu 24.04 runners (`ubuntu-24.04`,
`ubuntu-24.04-arm`), so every file needs glibc 2.39 (Ubuntu 24.04, Debian 13,
Fedora 40, current Arch) instead of the newest glibc of an Arch build.

Local test on Arch, from a clean checkout:

```sh
dist=$(mktemp -d)
exe=$(HOKUM_RELEASE_DIST=$dist scripts/build-linux.sh)
scripts/package-linux.sh "$exe" <version> x64 "$dist"
# The URLs only need the right shape; the staged files are used as they are.
url=https://anoma.ly/release/hokum/files/$(sha256sum < "$exe" | cut -c1-64)/hokum
scripts/release-aur.sh stage <version> "$exe" "$url" "$exe" "$url" "$dist/aur"
scripts/release-aur.sh test "$dist/aur"
```

## Secrets and setup (`release` environment, limited to `dev` and `production`)

| Name | Use |
| --- | --- |
| (none) | publishing to the release service: the job's GitHub OIDC token |
| `AUR_SSH_PRIVATE_KEY` | pushes `hokum-bin` (key `github-actions-hokum-aur` on Dax's AUR account); check it with **Hokum AUR key check** |
| `GITHUB_TOKEN` (built in) | tags and creates the GitHub release (`contents: write` in the `github` job only) |

Repository settings, as on Hex: default branch `dev`; rulesets "no main"
(refuses creating `main`), "production" (no deletion or force push), and
"release boundary (.github needs Dax)".
