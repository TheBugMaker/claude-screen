// How this mod learns that the conversation was swapped for another (`/clear`, `/resume`,
// `/branch`), to fold it afresh. Claude Code says so to a `classic.SessionStart` hook, but where an
// organization manages Claude Code its guard (`cc-plugin-sec-default`) sends `classic.*` events on
// past every mod the user installs, so there are two ways, picked by the mod's `signals` option:
// `classic.ts` and `guarded.ts`. Each registers its hooks with a function of its own (the hooks
// loader takes `on` only that way) and implements this for the hooks they share.
export interface Swaps {
  /** True when the conversation was swapped since the last ask, for the runs to be built again. */
  take(): boolean
}
