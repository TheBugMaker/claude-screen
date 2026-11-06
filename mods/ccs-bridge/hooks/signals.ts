// How this mod learns what Claude Code says only to `classic.*` hooks: that a permission dialog
// waits for you, and that work left behind will start the next turn. Where an organization manages
// Claude Code, its guard (`cc-plugin-sec-default`) sends `classic.*` events on past every mod the
// user installs, so there are two ways, picked by the mod's `signals` option: `classic.ts` and
// `guarded.ts`. The hooks they share tell the way in use what they see.
//
// The hooks loader takes `$` and `on` only as arguments of functions named at the top of a file:
// so a way's own hooks are a function of its file, and what it does with `$` is in `dialogs.ts`.
export interface Signals {
  /**
   * Call `id` was put to the mode's decider, for `approver` to answer where known. How long until
   * it counts as a dialog shown, in milliseconds; undefined when this way learns of its dialog
   * otherwise, or none shows.
   */
  ask(id: string, approver: 'host' | 'person' | undefined): number | undefined
  /** Call `id` is settled: it runs, or it was refused. */
  settle(id: string): void
  /** The turn ended, answered or interrupted. */
  end(): void
}
