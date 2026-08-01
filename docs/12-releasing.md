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

## Three version numbers, three jobs

Do not bump a schema because the app is being released:

| Number | Current source | Bump when |
| --- | --- | --- |
| Application version | The four manifest/lock locations above | Shipping a Wobu release |
| Project schema version | `wobu_core::SCHEMA_VERSION` | The canonical on-disk project format changes, with compatible loading or a migration plan |
| Index schema version | `wobu_store::index::INDEX_VERSION` | The disposable SQLite index layout or indexed interpretation changes |

The index can be rebuilt, so its version is independent of project compatibility. The project
schema governs whether a project folder can be opened and must not be used as a release counter.
All three values are exposed separately in the About panel and printed by the release-version
check; that visibility is not permission to keep them numerically aligned.
