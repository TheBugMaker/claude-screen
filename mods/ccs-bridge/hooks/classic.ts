// Learns from the `classic.*` hooks, as each dialog shows and each turn ends: exact, where they run.
import type { On } from 'claude-code'

import { show } from './dialogs'
import { report } from './report'
import type { Signals } from './signals'

/** Tool calls put to the mode's decider and not settled yet, oldest first. */
const asked: string[] = []

/** This way's own hooks. */
export function classic(on: On) {
  // Fires as the dialog shows, for the oldest call still asking.
  on('classic.PermissionRequest', ($, e, next) => {
    show($, asked.shift())
    return next(e)
  })
  // Just before an answered turn completes; an interrupted one has none. Background work or a
  // scheduled wakeup still to come starts the next turn without you.
  on('classic.Stop', async ($, e, next) => {
    const result = await next(e)
    // A Stop hook that blocks keeps the turn going.
    const wakes = (e.background_tasks?.length ?? 0) + (e.session_crons?.length ?? 0)
    if (e.agent_id === undefined && result.block === undefined && wakes > 0) {
      report($, { ev: 'asleep' })
    }
    return result
  })
}

export const classicSignals: Signals = {
  ask(id) {
    // Its dialog is `classic.PermissionRequest`'s to report.
    asked.push(id)
    return undefined
  },
  settle(id) {
    const index = asked.indexOf(id)
    if (index !== -1) asked.splice(index, 1)
  },
  end() {
    asked.length = 0
  },
}
