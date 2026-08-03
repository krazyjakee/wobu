/**
 * A read-only project folder, in one place.
 *
 * `paths::is_writable()` answers this once, when the project is opened, so it
 * cannot change under a session: either the folder is writable or the whole
 * workspace is a reader. Everything that follows from that is gathered here
 * rather than spread across the controls it turns off.
 *
 * The explanation is **one banner, raised once** when the project opens. A
 * tooltip on every disabled control would be the same sentence twenty times
 * over and still leave the user assembling the rule themselves — and a disabled
 * button with no reason at all reads as a bug rather than as a folder.
 *
 * What it switches off: creating, renaming, moving, deleting and duplicating
 * nodes, editing notes, autosave, and undo/redo — every one of which ends in a
 * write that the backend would reject anyway, only later and one action at a
 * time. Enhance and Generate join that list at their own buttons,
 * because they write to the node like any other edit; see `Editor.tsx`.
 */
export const READ_ONLY_BANNER = 'project.read_only'

export const READ_ONLY_TEXT =
  'This share is mounted read-only, so nothing can be written back to it. The world is fully ' +
  'readable — creating, renaming, moving, deleting and editing notes are switched off, and ' +
  'nothing is being autosaved.'
