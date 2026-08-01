# Packaging and releases

Wobu's beta releases are built by GitHub Actions from a version tag. A valid tag produces native
Tauri bundles on Linux, macOS, and Windows and uploads them to a **draft** GitHub release. A
maintainer inspects the three jobs and their assets before publishing the draft.

## Distribution and signing decision

Beta releases are **unsigned manual downloads**. Wobu does not currently hold an Apple Developer
ID certificate, an Apple notarisation credential, or a Windows Authenticode certificate. The
release workflow deliberately contains none of those secrets, so it neither implies a trust chain
we do not have nor quietly depends on one maintainer's machine.

This is a temporary distribution decision, not a claim that unsigned installers are equivalent to
signed ones. macOS Gatekeeper and Windows SmartScreen will warn, and users must verify that the
download came from this repository's GitHub Releases page before bypassing a warning. Do not copy
an unsigned bundle to another download host.

The app also uses **manual updates**. There is no Tauri updater plugin, endpoint, updater JSON, or
updater signing keypair. The release action explicitly disables `latest.json`; because no updater
plugin or signing key is configured, no signed updater bundle is produced. Users install a newer
release over the old one; projects are ordinary folders and are not stored inside the application
bundle.

Before calling releases stable, revisit both decisions together:

1. Obtain Apple Developer ID and Windows code-signing credentials, put them in GitHub Actions
   encrypted secrets, sign Windows installers, and sign plus notarise macOS bundles.
2. Add and permission the Tauri updater plugin, generate an offline updater signing keypair, store
   only the private key and password in Actions secrets, commit the public key and HTTPS endpoint,
   and enable signed updater artifacts in the release workflow.
3. Test upgrade and rollback behaviour on every supported platform. Never enable an updater that
   accepts unsigned payloads or commit its private key.

## Installing an unsigned beta

Only download assets attached to a release at
`https://github.com/krazyjakee/wobu/releases`. Compare the release tag with the version shown in
Wobu's About panel.

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
version. It creates a draft even after successful builds. Before publishing, confirm that all three
matrix jobs passed, the draft contains installers from all three operating systems, and its
unsigned-install warning is intact.

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
and refuses drift, missing icons, disabled bundling, or accidental updater activation. Its focused
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

Tauri writes artifacts below `src-tauri/target/release/bundle/`.

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
