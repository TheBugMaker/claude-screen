// Learns of a swap from `classic.SessionStart`, once the new conversation is there: exact, where it
// runs.
import type { On } from 'claude-code'

import { rebuild } from './register'
import type { Swaps } from './swaps'

/** This way's own hooks. */
export function classic(on: On) {
  // `/resume`, `/clear` and a compaction change the conversation without a `session.start`.
  on('classic.SessionStart', async ($, e, next) => {
    const result = await next(e)
    if (e.source !== 'startup') rebuild($)
    return result
  })
}

/** Its hook builds the runs again itself. */
export const classicSwaps: Swaps = { take: () => false }
