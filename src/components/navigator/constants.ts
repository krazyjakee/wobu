/** Values the Navigator, its menus and its rows all need. */

export const DRAG_MIME = 'application/x-wobu-node'

/**
 * Why every writing control in the navigator is refused at once.
 *
 * Said in one place because a user who meets it on one button and then another
 * should be told the same thing, and because it is a *precondition* — it names
 * what would have to change for the button to work — rather than a restatement
 * of the fact that the button does not work.
 */
export const READ_ONLY_REASON =
  'This project folder is read-only: Wobu cannot write to it, so nothing can be created here. Check the folder permissions, or reopen a copy somewhere writable.'
