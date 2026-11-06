// The permission dialogs ccs hears of: each as it shows, and its answer as its call settles.
import type { EngineInterface, Timer } from 'claude-code'

import { report } from './report'

/** The calls whose dialog was reported. */
const shown = new Set<string>()
/** Calls whose dialog is reported when their timer fires, unless they settle first. */
const pending = new Map<string, Timer>()

/** A dialog shows, for call `id` when that is known. */
export function show($: EngineInterface, id: string | undefined) {
  if (id !== undefined) shown.add(id)
  report($, { ev: 'permission' })
}

/** Call `id`'s dialog counts as shown if the call is still unsettled in `ms`. */
export function showAfter($: EngineInterface, id: string, ms: number) {
  const timer = $.clock.after(ms, () => {
    pending.delete(id)
    show($, id)
  })
  pending.set(id, timer)
}

/** Call `id` is settled: if its dialog was shown, it was answered yes or no. */
export function settle($: EngineInterface, id: string, yes: boolean) {
  pending.get(id)?.cancel()
  pending.delete(id)
  if (shown.delete(id)) report($, { ev: yes ? 'approved' : 'denied' })
}

/** The turn ended, answered or interrupted: an interrupt also closes the dialog. */
export function endTurn() {
  for (const timer of pending.values()) timer.cancel()
  pending.clear()
  shown.clear()
}
