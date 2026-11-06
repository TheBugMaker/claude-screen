// Learns without the `classic.*` hooks, which the organization's guard keeps from this mod: a dialog
// from an ask still unsettled after a while, and never that work left behind will wake Claude, so
// `asleep` is not reported. It has no hooks of its own.
import type { Signals } from './signals'

/**
 * How long an ask goes unsettled before it counts as a dialog waiting for you. `tool.check` says
 * `ask` before anyone answers, and in auto mode Claude Code answers most asks itself, with no
 * dialog, in under a second (0.1 to 0.9 s on 2.1.316); its classifier takes a model call.
 */
const DIALOG_MS = 3000

export const guardedSignals: Signals = {
  // Else a host answers it, and nothing waits for you. A plan or a question always asks a person.
  ask: (id, approver) => (approver === 'person' ? DIALOG_MS : undefined),
  settle() {},
  end() {},
}
