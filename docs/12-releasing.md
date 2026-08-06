# Packaging and releases

Wobu's beta releases are built by GitHub Actions from a version tag. A valid tag produces native
Tauri bundles on Linux, macOS, and Windows and uploads them to a **published** GitHub release. The
tag is the whole release request: there is no second manual act, because a release that needs one
is a release that installed copies poll and silently find nothing at.

## Distribution and signing decision

Beta releases are **unsigned manual downloads**. Wobu does not currently hold an Apple Developer
ID certificate, an Apple notarisation credential, or a Windows Authenticode certificate. The
release workflow deliberately contains none of those secrets, so it neither implies a trust chain
we do not have nor quietly depends on one maintainer's machine.

This is a temporary distribution decision, not a claim that unsigned installers are equivalent to
signed ones. macOS Gatekeeper and Windows SmartScreen will warn, and users must verify that the
download came from this repository's GitHub Releases page before bypassing a warning. Do not copy
an unsigned bundle to another download host.

Updates, by contrast, **are** signed. Wobu ships the Tauri updater plugin, and the release workflow
signs each updater payload with an offline keypair whose public half is committed in
`src-tauri/tauri.conf.json`. The two are separate trust roots and should not be conflated: code
signing would establish publisher identity to the *operating system* at install time, which Wobu
still does not have; updater signing establishes that a payload came from this repository's key,
which it does. A client refuses any update it cannot verify against the committed public key, so a
replaced `latest.json`, a hostile mirror or a tampered release asset produces a refusal rather than
an install.

Before calling releases stable, the remaining decision is code signing:

1. Obtain Apple Developer ID and Windows code-signing credentials, put them in GitHub Actions
   encrypted secrets, sign Windows installers, and sign plus notarise macOS bundles.
2. Test upgrade and rollback behaviour on every supported platform. Never accept an unsigned
   updater payload or commit the updater private key.

## The updater

Installed copies check **only when a user presses the button** in Settings → Updates. Nothing runs
at startup and nothing polls: opening Wobu is not consent to contact GitHub, which is the same
promise the privacy policy makes about every other network destination.

The moving parts, all of which `npm run release:check` verifies together:

- `bundle.createUpdaterArtifacts: true` in `src-tauri/tauri.conf.json` — produces the signed
  payload beside each bundle.
- `plugins.updater.endpoints` — a single HTTPS URL,
  `https://github.com/krazyjakee/wobu/releases/latest/download/latest.json`. `releases/latest`
  excludes prereleases by design, so a tag containing `-` never offers itself to stable users.
- `plugins.updater.pubkey` — the public half of the signing keypair. Committed on purpose; it is
  what makes the trust root this repository rather than the HTTPS connection.
- `includeUpdaterJson: true` in the release workflow — attaches `latest.json` to the release.
  `releases/latest` does not resolve against a draft, which is why the workflow publishes outright.
- `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` as repository Actions
  secrets. The workflow fails fast if the private key is absent rather than publishing a release
  whose payloads every installed client would then reject.

Only self-replacing bundle formats update in place: AppImage on Linux, `.app`/`.dmg` on macOS, and
NSIS on Windows. A `.deb` or `.msi` is owned by its installer, and the Settings pane says so and
points at the releases page instead.

### Rotating or regenerating the signing key

Generate a keypair outside the repository and never commit the private half:

```sh
npm run tauri signer generate -- -w ~/.wobu/updater.key
```

Put the contents of `~/.wobu/updater.key` in the `TAURI_SIGNING_PRIVATE_KEY` repository secret and
its password (empty if none) in `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, then commit the contents of
`~/.wobu/updater.key.pub` as `plugins.updater.pubkey`.

Rotation is a **breaking** operation: already-installed copies carry the old public key and will
reject anything signed with the new one. They are not broken, but they will stop finding updates
and their users must download a new bundle by hand. Losing the private key has the same effect and
cannot be undone. Rotate only in response to a suspected compromise, and say plainly in the release
notes that a manual reinstall is required.

## Installing an unsigned beta

Only download assets attached to a release at
`https://github.com/krazyjakee/wobu/releases`. Compare the release tag with the version shown in
Wobu's About panel. This applies to a *first* install; once Wobu is running, **Settings → Updates**
installs later releases in place and verifies each one's signature first.

- **Linux:** prefer the package for your distribution. For an AppImage, make it executable with
  `chmod +x Wobu_*.AppImage`, then run it. Linux packages are not repository-signed, so the package
  manager cannot establish publisher identity.
- **macOS:** drag Wobu into Applications. On first launch, control-click Wobu and choose **Open**,
  then confirm **Open**. If macOS reports the app as damaged rather than offering that choice, and
  only after confirming the bundle came from the releases page, run
  `xattr -dr com.apple.quarantine /Applications/Wobu.app` and open it again.
- **Windows:** launch the installer. If Microsoft Defender SmartScreen appears, check that the
  publisher is shown as unknown, select **More info**, then **Run anyway**. An unexpected named
  publisher is a reason to stop, not an improvement.

## Preparing a release

Application versions use SemVer and tags have a `v` prefix, for example `v0.2.0`. Five committed
files describe the same application release:

- `package.json`;
- the generated version entries in `package-lock.json`;
- `src-tauri/tauri.conf.json`, used in bundle metadata;
- `[workspace.package].version` in `src-tauri/Cargo.toml`, inherited by the app and displayed by the
  About panel;
- Wobu workspace package entries in `src-tauri/Cargo.lock`.

To prepare `0.2.0`, from the repository root:

```sh
npm version 0.2.0 --no-git-tag-version
# Set version = "0.2.0" in src-tauri/tauri.conf.json.
# Set [workspace.package].version = "0.2.0" in src-tauri/Cargo.toml.
cargo metadata --manifest-path src-tauri/Cargo.toml --format-version 1 --no-deps > /dev/null
node .github/scripts/verify-release-version.mjs v0.2.0
```

Review and commit all five manifest/lock files before tagging. Create the tag only on the reviewed
release commit and push it:

```sh
git tag -a v0.2.0 -m "Wobu v0.2.0"
git push origin v0.2.0
```

The workflow rejects malformed tags and any tag that disagrees with a committed application
version, and publishes the release as soon as the bundles are built. Pushing the tag is therefore
the irreversible step — check the version stamps before it, not the assets after.

Afterwards, confirm that all three matrix jobs passed and that the release carries installers from
all three operating systems. `fail-fast: false` means a single broken platform still publishes,
just without that platform's installer; a client only updates to what `latest.json` lists, so the
result is no update offered there rather than a bad one. Fix it by re-tagging a patch version.

## Implementation and documentation close-out

Before closing an implementation issue or including it in a release, use this lightweight checklist
in the same change (write “not affected” explicitly rather than silently skipping an item):

- [ ] Update the feature's status and canonical issue link in [the roadmap](09-roadmap.md). Use
  **validated** only when an acceptance pass actually records evidence.
- [ ] Walk the user-facing flow in [`docs/guide`](guide/). Describe only controls that are reachable
  in the current UI; label retained future steps **planned** and link their open issue.
- [ ] Add or update repeatable contracts and the honest manual smoke boundary in
  [acceptance evidence](13-acceptance-evidence.md).
- [ ] Check Settings, inline help, empty states, tooltips, and shortcut tables for stale capability
  claims.
- [ ] Put the issue link beside any retained requirement in planning documents. Closed/open issue
  state is authoritative when prose and tracker disagree.

The implementation issue should not close with contradictory public guidance merely because the
binary is correct. Conversely, documentation alone is not acceptance evidence for a runtime flow.

## Version audit tooling

[`release/versions.json`](../release/versions.json) is the release audit record:

- `appVersion` is SemVer shown in About and embedded in installers. It must match `package.json`,
  `package-lock.json`, `src-tauri/tauri.conf.json`, the Cargo workspace version, and every Wobu
  workspace package in `Cargo.lock`.
- `projectSchemaVersion` is the canonical `.wobu` folder format. Change it only with an explicit
  compatibility/migration decision; a routine app release must not touch it.
- `indexSchemaVersion` is the disposable local SQLite layout. Change it only when the index schema
  changes; opening then rebuilds the cache. It does not imply a project format change.

Set only the app version with:

```sh
npm run release:set -- 0.2.0
npm run release:check
```

The stamping tool serialises and stages every manifest beside its target before replacing files one at
a time. If an operation reports an error, it rolls back files already replaced. This is not a durable
cross-file transaction: a process or operating-system crash can interrupt publication or rollback and
leave mixed versions. Always run it from a clean git checkout; after an interrupted process, inspect
`git status`, restore the manifest set from git if necessary, and run the stamp again. The tool updates
the JavaScript, Tauri, Cargo and lockfile app versions together, but never edits either schema
constant. For an intentional schema change, edit the corresponding Rust constant and
`release/versions.json` in the same reviewed change. `npm run release:check` prints all three numbers
and refuses drift, missing icons, disabled bundling, or an updater that has lost its artifacts,
its HTTPS endpoint, or its public key. Its focused
static tests are available as `npm run release:tool:test`.

## Local fallback release procedure

The tag workflow is the normal publication path. To reproduce or diagnose one of its jobs locally,
start from a clean checkout of the intended tag on the matching native host. Install the repository's
pinned Rust toolchain, Node dependencies with `npm ci`, and the current [Tauri platform
prerequisites](https://v2.tauri.app/start/prerequisites/). Then:

1. Run `npm run release:check` and record its three-number output.
2. Run the normal project checks required by the release owner.
3. Build on the target operating system with the commands below. Native builds are the supported
   path; cross-compiling Windows is explicitly a last resort in Tauri's documentation.
4. Launch the installed application, create/open a disposable project, and confirm About shows the
   intended app/project/index versions.
5. Generate SHA-256 files beside every public artifact and verify them on a second machine.
6. Create a manual GitHub Release named `v<appVersion>`, attach artifacts and checksums, and state
   prominently that the bundles are unsigned.

Tauri writes artifacts below `src-tauri/target/release/bundle/`. The `--no-sign` in each command
below is what lets a local build proceed without the updater private key; those artifacts therefore
carry no update signature and must not be published as an update — a diagnostic build is not a
release.

### Linux

Build on the oldest Linux distribution the release intends to support so the linked system libraries
do not silently raise the runtime floor:

```sh
npm run tauri build -- --bundles deb,appimage --no-sign
sha256sum src-tauri/target/release/bundle/deb/* src-tauri/target/release/bundle/appimage/* \
  > SHA256SUMS-linux.txt
```

Publish the `.deb`, AppImage, and checksum file. Linux package signing remains optional external work;
checksums provide integrity only when users obtain them from the authenticated release page. Users
can install the Debian package with their graphical package manager or `sudo apt install` followed by
the downloaded local `.deb` path. The AppImage is portable: run `chmod +x` on the downloaded AppImage,
then launch it directly. Exact filenames come from the release assets.

### macOS

Build each architecture intended for release on macOS (or make an explicitly tested universal build):

```sh
npm run tauri build -- --bundles app,dmg --no-sign
shasum -a 256 src-tauri/target/release/bundle/dmg/*.dmg > SHA256SUMS-macos.txt
```

Publish the `.dmg` and checksum. Because it is not Developer ID signed or notarised, Gatekeeper may
block the first launch. Users should drag Wobu to Applications, try to open it once, then use **System
Settings → Privacy & Security → Open Anyway** and confirm. Do not tell users to disable Gatekeeper
globally or run a blanket `xattr` command. A public friction-free macOS release remains blocked on an
Apple Developer ID Application certificate, notarisation credentials and a trusted macOS release
machine. Tauri documents that direct-download macOS distribution requires signing and notarisation.

### Windows

Build on Windows so the supported MSVC toolchain, WiX and NSIS paths are used:

```powershell
npm run tauri build -- --bundles msi,nsis --no-sign
$artifacts = Get-Item src-tauri\target\release\bundle\msi\*.msi,
  src-tauri\target\release\bundle\nsis\*.exe
$artifacts | Get-FileHash -Algorithm SHA256 |
  ForEach-Object { "$($_.Hash.ToLower())  $([IO.Path]::GetFileName($_.Path))" } |
  Set-Content SHA256SUMS-windows.txt
```

Publish both installers and checksums. Windows SmartScreen will identify an unsigned browser download
as untrusted and show **Unknown publisher**. Users must compare the filename/version and checksum,
choose **More info → Run anyway**, then continue the installer. Do not weaken SmartScreen globally.
Removing this warning remains blocked on a current Windows code-signing certificate or managed signing
service and a trusted Windows release machine.

## Signing handoff

When external identities exist, keep private material out of git and prefer environment/OS key-store
integration described by Tauri's [macOS signing](https://v2.tauri.app/distribute/sign/macos/) and
[Windows signing](https://v2.tauri.app/distribute/sign/windows/) guides. A signed release is complete
only after signatures are verified on the final downloadable bytes and macOS notarisation is stapled;
having configuration fields ready is not evidence that either happened.
