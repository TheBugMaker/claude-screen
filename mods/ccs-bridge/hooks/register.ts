// Reports what this Claude session does to the ccs that runs it, which reads the reports in
// `crates/ccs-claude/src/bridge.rs`. Started by anything else, it does nothing.
import type { EngineInterface, Register } from 'claude-code'

/** One report; `ev` names it. */
type Report =
  | { ev: 'ready'; session_id: string; root: string }
  | { ev: 'submitted' | 'working' | 'permission' | 'approved' | 'denied' | 'asleep' | 'idle' }
  | { ev: 'title'; title: string }
  | { ev: 'root'; root: string }

/** Tool calls put to the mode's decider and not settled yet, oldest first. */
const asked: string[] = []
/** The calls whose permission dialog is shown. */
const shown = new Set<string>()
/** Where the session works, as last reported: the worktree Claude made for it, if it did. */
let root: string | undefined
/** The reports leave one at a time, so ccs reads them in order. */
let sent: Promise<unknown> = Promise.resolve()

function report($: EngineInterface, event: Report) {
  sent = sent.then(() => send($, event)).catch(() => undefined)
}

async function send($: EngineInterface, event: Report) {
  const exe = await $.env.get('CCS_EXE')
  const socket = await $.env.get('CCS_SOCKET')
  const session = await $.env.get('CCS_SESSION')
  if (exe === undefined || socket === undefined || session === undefined) return
  await $.process.run([exe, 'hook', socket, session], { stdin: JSON.stringify(event) })
}

/** Call `id` is settled: if its dialog was shown, it was answered yes or no. */
function settled($: EngineInterface, id: string | undefined, yes: boolean) {
  if (id === undefined) return
  const index = asked.indexOf(id)
  if (index !== -1) asked.splice(index, 1)
  if (shown.delete(id)) report($, { ev: yes ? 'approved' : 'denied' })
}

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    root = await $.session.root()
    report($, { ev: 'ready', session_id: await $.session.id(), root })
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
  // `ask` goes to the dialog, or in auto mode to the classifier, which shows no dialog.
  on('tool.check', async ($, e, next) => {
    const verdict = await next(e)
    if (verdict.decision === 'ask' && e.tool_use_id !== undefined) asked.push(e.tool_use_id)
    return verdict
  })
  // Fires as the dialog shows, for the oldest call still asking.
  on('classic.PermissionRequest', ($, e, next) => {
    const id = asked.shift()
    if (id !== undefined) shown.add(id)
    report($, { ev: 'permission' })
    return next(e)
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
  // Answered or interrupted; an interrupt also closes the dialog.
  on('turn.complete', ($, e, next) => {
    if (e.agentId === undefined) {
      asked.length = 0
      shown.clear()
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
}
