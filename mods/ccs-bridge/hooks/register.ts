// Reports what this Claude session does to the ccs that runs it, which reads the reports in
// `crates/ccs-claude/src/bridge.rs`. Started by anything else, it does nothing.
import type { EngineInterface, Register } from 'claude-code'

import { classic, classicSignals } from './classic'
import { endTurn, settle, showAfter } from './dialogs'
import { guardedSignals } from './guarded'
import { report } from './report'
import type { Signals } from './signals'

/** How much of a `/plan` prompt Haiku titles: the task, not a file pasted after it. */
const TITLED_CHARS = 120
/** How often, and how many times, the draft is tried while the prompt box isn't up yet. */
const DRAFT_EVERY_MS = 100
const DRAFT_TRIES = 50

/** Where the session works, as last reported: the worktree Claude made for it, if it did. */
let root: string | undefined
/** The session has started once: a later start, as after /clear, leaves the prompt box alone. */
let drafted = false
/** How this mod learns of dialogs and of work left behind, as its `signals` option picks. */
let signals: Signals = classicSignals

/** Has Haiku title the task in `prompt`, for ccs to show until Claude titles the session. */
async function guess($: EngineInterface, prompt: string) {
  const reply = await $.model.complete({
    model: 'haiku',
    system:
      'Reply with a title of two to six words for the task below, in sentence case, with no quotes and no final period.',
    prompt,
    effort: 'low',
    maxTokens: 30,
    timeoutMs: 15_000,
  })
  const title = reply.isAnswered ? reply.text.trim().split('\n')[0] : undefined
  if (title) report($, { ev: 'guess', title })
}

/**
 * Puts `CCS_DRAFT`, the prompt ccs started this Claude with, in the prompt box for you to edit
 * or send: a merge session's, to fix the conflicts. Again a little later while there is no box
 * yet to take it.
 */
async function draft($: EngineInterface, tries: number) {
  const text = await $.env.get('CCS_DRAFT')
  if (!text) return
  const { isFilled } = await $.prompt.fill({ text })
  if (!isFilled && tries > 1) {
    $.clock.after(DRAFT_EVERY_MS, () => void draft($, tries - 1).catch(() => undefined))
  }
}

/** Call `id` is settled: if its dialog was shown, it was answered yes or no. */
function settled($: EngineInterface, id: string | undefined, yes: boolean) {
  if (id === undefined) return
  signals.settle(id)
  settle($, id, yes)
}

export const register: Register = (on, options) => {
  // Whether Claude Code's `classic.*` hooks reach this mod: see `signals.ts`.
  const isGuarded = options.signals === 'guarded'
  signals = isGuarded ? guardedSignals : classicSignals
  if (!isGuarded) classic(on)

  on('session.start', async ($, e, next) => {
    root = await $.session.root()
    report($, { ev: 'ready', session_id: await $.session.id(), root })
    // Once the session has started; and only the first time, so it never replaces what you typed.
    if (!drafted) {
      drafted = true
      $.clock.after(0, () => void draft($, DRAFT_TRIES).catch(() => undefined))
    }
    return next(e)
  })
  on('prompt.submit', async ($, e, next) => {
    const result = await next(e)
    // Typed by you; a plugin's or a background task's prompt is not.
    if (e.origin.kind === 'composer' && result.drop === undefined) {
      report($, { ev: 'submitted' })
    }
    return result
  })
  on('turn.start', async ($, e, next) => {
    const result = await next(e)
    if (e.agentId === undefined) report($, { ev: 'working' })
    return result
  })
  // `ask` goes to the mode's decider: the dialog, or in auto mode the classifier, which shows none.
  on('tool.check', async ($, e, next) => {
    const verdict = await next(e)
    const id = e.tool_use_id
    if (verdict.decision === 'ask' && id !== undefined) {
      const wait = signals.ask(id, e.approver)
      if (wait !== undefined) showAfter($, id, wait)
    }
    return verdict
  })
  // Nothing fires when you answer yes, but the call's row starts running right away.
  on('ui.render', { component: 'ToolUse' }, ($, e, next) => {
    if (e.props.isRunning) settled($, e.props.tool_use_id, true)
    return next(e)
  })
  on('tool.call', async ($, e, next) => {
    const result = await next(e)
    // A no at the dialog ends the call in an error.
    settled($, e.tool_use_id, result.deny === undefined && result.isError !== true)
    // EnterWorktree and ExitWorktree move the session, and no other event says so.
    const now = await $.session.root()
    if (root !== undefined && now !== root) {
      root = now
      report($, { ev: 'root', root })
    }
    return result
  })
  // Answered or interrupted; an interrupt also closes the dialog.
  on('turn.complete', ($, e, next) => {
    if (e.agentId === undefined) {
      signals.end()
      endTurn()
      report($, { ev: 'idle' })
    }
    return next(e)
  })
  on('command.run', { command: 'rename' }, async ($, e, next) => {
    const result = await next(e)
    const title = e.args.trim()
    if (title !== '') report($, { ev: 'title', title })
    return result
  })
  // Claude titles a session started with `/plan <prompt>` only once the plan is approved. The
  // guess is made on a timer, so the plan doesn't wait for it and it outlives this command.
  on('command.run', { command: 'plan' }, async ($, e, next) => {
    const result = await next(e)
    const prompt = [...e.args.trim()].slice(0, TITLED_CHARS).join('')
    if (prompt !== '') $.clock.after(0, () => void guess($, prompt).catch(() => undefined))
    return result
  })
}
