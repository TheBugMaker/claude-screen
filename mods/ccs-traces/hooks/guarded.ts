// Learns of a swap without `classic.SessionStart`, which the organization's guard keeps from this
// mod: `session.end` says one starts, before the new conversation is there, so the runs are built
// again at its first row. A compaction goes unseen, which leaves the runs as they were: the rows
// they are built from stay as they were too.
import type { On } from 'claude-code'

import type { Swaps } from './swaps'

/** A swap started that the runs were not built again for yet. */
let swapped = false

/** This way's own hooks. */
export function guarded(on: On) {
  // `/branch` ends the session as `resume` does.
  on('session.end', ($, e, next) => {
    if (e.reason === 'clear' || e.reason === 'resume') swapped = true
    return next(e)
  })
}

export const guardedSwaps: Swaps = {
  take() {
    const was = swapped
    swapped = false
    return was
  },
}
