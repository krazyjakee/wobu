# Portable narrative storage

Canonical narrative work lives in the project folder. SQLite contains a derived document index,
including source hashes, names, identities, text/lifecycle fields and explicit parse errors. Deleting
that local index or copying the project to another machine rebuilds these inputs from files.
Art-only projects require no narrative directory or project schema migration.

## File registry and records

The registry lists only these flat canonical paths:

| Path | Meaning | Write policy |
| --- | --- | --- |
| `narrative/scenes/<slug>.yaml` | Scene graph, accepted/authored text, stable line/variant IDs and editorial state | Guarded |
| `narrative/state.yaml` | Typed variable declarations | Guarded |
| `narrative/world.yaml` | Facts, knowledge, relationships, events, quests and restrictions | Guarded |
| `narrative/scenarios/<id>.json` | Scenario payload | Guarded |
| `narrative/proposals/<id>.json` | Proposed content payload | Guarded |
| `narrative/receipts/<id>.json` | Immutable generation receipt payload | Immutable |
| `narrative/policies/<id>.json` | Policy/approval payload | Guarded |
| `narrative/production/<id>.json` | Later localisation/audio/production payload | Guarded |
| `narrative/objects/<hash>.json` | Internal immutable record object | Immutable |
| `narrative/receipt-bindings/<id>.json` | Internal receipt identity binding | Immutable |
| `narrative/publications/<id>.json` | Complete multi-record revision manifest | Guarded |
| `narrative/deletions/<id>.json` | Explicit deletion and recoverable original bytes | Immutable |

Replaceable JSON records use a strict envelope with `schema_version: 1`, stable `id`, `kind`, `name`
and object `payload`. The owning domain validates that payload before writing it. Storage validates
identity, path, envelope version and unknown envelope fields. These storage slots do not implement
provider generation, proposal review, production jobs or any N5 behaviour.

Read stamps travel with the record; they are not recovered from SQLite at save time. A stale save
parks the proposed bytes as a named conflict sibling beside the current file. Unsupported existing
versions cannot be downgraded by resaving an older envelope. Scene/state/world retain their existing
versioned YAML formats and guarded source APIs.

All registered paths are normalized project-relative paths. Existing symbolic links in any component
are refused. Layout, recovery copies, local staging/indexes and credentials are outside canonical
record discovery. The accepted-source fingerprint remains scene/state/world only; scenario and
proposal bookkeeping does not change the game compiler's input fingerprint. Watch reconciliation
still observes every registered record kind, including external edits and local file removals.

## Publication and receipt identities

`publish_narrative_records` writes immutable dependencies first and changes the publication manifest
last, through one guarded write. Readers follow only a validated complete manifest, not a directory
of objects. An interruption leaves the previous complete revision visible and may leave unused
internal objects. Missing objects, changed hashes, mismatched identities or invalid receipt bindings
make a publication explicitly incomplete. Index entries have a `visible` flag: internal objects,
bindings, deletion metadata and incomplete publications are never semantic query results.

Receipt identity is immutable across standalone and published forms. Its binding covers the canonical
JSON bytes of the entire typed record envelope: version, ID, kind, name and payload, with pretty JSON
and a trailing LF. Both write routes reserve the same binding before publication and reject another
payload claiming that identity. A preexisting standalone receipt without a binding is checked against
its actual canonical bytes first. Deleting or recreating a presentation/reference to a receipt must
not delete its identity binding.

These publication rules provide a storage boundary for later jobs; they do not treat several
independent scene edits as one transaction. A consumer requiring a coherent set of record revisions
must use a publication manifest and validate all its references.

## Delete and restore

Explicit deletions retain the supported original bytes and their hash in an immutable deletion record
before moving the file into `narrative/recovery/`. The operation checks the original stamp and never
deletes a different current revision. If a writer wins during the move, its captured bytes are kept
in recovery and restored with a guarded write; a further winner is preserved as well. Local folder
removal changes derived indexes but is never inferred to be a peer deletion.

The recovery API lists deletion names and project-relative targets. Restore writes an immutable
`narrative/restorations/<id>.json` marker referencing exactly one deletion ID, then restores its
original bytes without overwriting a newer current file. A newer file produces the usual conflict
sibling containing the retained original. `restored` means the explicit restoration was requested;
it does not assert that old text won a conflict. A later deletion gets a fresh identity and is not
revoked by an earlier restoration.

Startup recovery and reconciliation finish interrupted deletion/restore operations from these
portable records. Replaying them is idempotent. Copying exact old source bytes back into the folder
cannot undo an active deletion; use the explicit restore operation. Immutable receipt bindings are
never deletion targets, and restoring a receipt must agree with its original identity binding.
Read-only projects list recovery history but refuse mutations.

## Verification

Storage tests cover each record kind, guarded concurrent edits, immutable receipt reuse across both
write routes, schema/identity rejection, internal orphan objects, incomplete publications, a full
project-folder copy with a new machine-local index, index deletion/rebuild, external edits/removals,
and symlink refusal. Existing scene tests cover read-only shares and unchanged art-only project open.

## Desktop recovery

Open **Narrative → Recovery…** to inspect retained deletions by their saved name and project-relative
path. **Restore** explicitly requests the retained original. The dialog never uploads or renders the
original source bytes just to list history. Read-only projects may inspect history but cannot restore.

A successful restore refreshes the scene/world/state queries. If a newer file occupies the same path,
it remains current and the original is parked as a conflict copy; the dialog reports the retained
copy's path and refreshes the ordinary conflict list. **Restoration requested** means a restoration
marker was recorded, including this conflict outcome. It does not imply the older content replaced
a newer edit. Completed history remains visible. **Retry restore** repeats the guarded request if the original
recovery was interrupted after recording its marker; it still preserves a newer current file.
**Refresh history** rereads canonical recovery metadata.

The [recovery screenshot](evidence/narrative-153/recovery.png) uses the real React dialog with
explicitly mocked browser IPC. Rust command tests separately verify metadata-only listing, original
byte restoration and preservation of both versions during a competing edit. This is not a native
engine or provider validation claim.
