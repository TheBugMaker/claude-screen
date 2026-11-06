// ccs-bridge where an organization manages Claude Code: its built-in guard, `sec-default`, sits
// outermost and sends some events on past every mod the user installed (code.claude.com/docs/en/
// plugins/mods/admin, "Know which hooks of a user's mod don't run"). GUARD stands in for it.
import type { On } from 'claude-code'
import type { Plugin } from 'claude-code/testing'
import { expect, mock, test } from 'claude-code/testing'

/** The guard's passing over, as the admin page lists it: what reaches no user mod. */
const GUARD: Plugin = {
  name: 'guard',
  tier: 'prepend',
  register(on) {
    on('classic.*', ($, e, next) => next.to(e, 'append'))
    on('session.append', { door: ['attachment', 'hook-context', 'compaction'] }, ($, e, next) =>
      next.to(e, 'append'),
    )
  },
}

/** What `ccs hook` answers: nothing. */
const RAN = {
  value: { exitCode: 0, stdout: '', stderr: '', isStdoutTruncated: false, isStderrTruncated: false },
}
/** What ccs gives the Claude it starts. */
const ENV = { CCS_EXE: '/opt/ccs', CCS_SOCKET: '/run/ccs.sock', CCS_SESSION: '3' }

/** The engine beneath the mod, for the events these tests raise. */
function engine(on: On) {
  on('classic.PermissionRequest', () => ({}))
  on('classic.Stop', () => ({}))
  on('turn.complete', () => ({ text: '' }))
}

test('the classic path learns of no dialog and no sleep under the guard', { plugins: [GUARD] }, async ($, on) => {
  engine(on)
  mock.env(on, ENV)
  const sent: unknown[] = []
  on('process.run', ($, e) => {
    sent.push(JSON.parse(e.init?.stdin ?? 'null'))
    return RAN
  })
  const clock = mock.clock(on)
  const shell = { id: 'b1', type: 'shell', status: 'running', description: 'Sleep', command: 'sleep 20' }

  await $.classic.PermissionRequest({
    tool_name: 'ExitPlanMode',
    tool_input: { plan: 'Do it' },
    permission_mode: 'plan',
  })
  await $.classic.Stop({ stop_hook_active: false, background_tasks: [shell], session_crons: [] })
  await $.turn.complete({ answer: 'ok', durationMs: 10, isAborted: false, reason: 'answer', turnId: 'turn-1' })
  await clock.settle()

  // Neither `permission` nor `asleep`: only the end of the turn gets through.
  expect(sent).toEqual([{ ev: 'idle' }])
})

/** The guarded path, as ccs sets it where the guard is. */
const GUARDED = { plugins: [GUARD], options: { signals: 'guarded' } }
/** A call's check as the engine raises it, a person to answer its ask: fields the engine sets. */
const asked = (tool_use_id: string, tool = 'Bash') =>
  ({ tool, input: { command: 'ls' }, tool_use_id, approver: 'person' }) as never
const END = { answer: '', durationMs: 10, isAborted: false, reason: 'answer' } as const

/** The engine beneath the mod for a call: `decide` says what its check decides. */
function calls(on: On, decide: () => 'allow' | 'ask') {
  engine(on)
  on('tool.check', () => ({ decision: decide() }))
  on('tool.call', () => ({ result: 'done' }))
  on('session.root', () => ({ value: '/work' }))
}

test('the guarded path reports a dialog still unanswered after 3 s, then its answer', GUARDED, async ($, on) => {
  let decision: 'allow' | 'ask' = 'ask'
  calls(on, () => decision)
  mock.env(on, ENV)
  const sent: unknown[] = []
  on('process.run', ($, e) => {
    sent.push(JSON.parse(e.init?.stdin ?? 'null'))
    return RAN
  })
  const clock = mock.clock(on)

  await $.tool.check(asked('t1', 'ExitPlanMode'))
  await clock.advance(2999)
  expect(sent).toEqual([])
  await clock.advance(1)
  expect(sent).toEqual([{ ev: 'permission' }])
  // You approve: the call runs.
  decision = 'allow'
  await $.tool.call({ tool: 'Bash', tool_use_id: 't1', command: 'ls' })
  await $.turn.complete({ ...END, turnId: 'turn-1' })
  await clock.settle()

  expect(sent).toEqual([{ ev: 'permission' }, { ev: 'approved' }, { ev: 'idle' }])
})

test('the guarded path reports nothing for an ask settled sooner, as auto mode settles one', GUARDED, async ($, on) => {
  let decision: 'allow' | 'ask' = 'ask'
  calls(on, () => decision)
  mock.env(on, ENV)
  const sent: unknown[] = []
  on('process.run', ($, e) => {
    sent.push(JSON.parse(e.init?.stdin ?? 'null'))
    return RAN
  })
  const clock = mock.clock(on)

  await $.tool.check(asked('t1'))
  await clock.advance(800)
  decision = 'allow'
  await $.tool.call({ tool: 'Bash', tool_use_id: 't1', command: 'ls' })
  await clock.advance(10_000)

  expect(sent).toEqual([])
})

test('the guarded path reports no dialog once the turn ends before it would', GUARDED, async ($, on) => {
  calls(on, () => 'ask')
  mock.env(on, ENV)
  const sent: unknown[] = []
  on('process.run', ($, e) => {
    sent.push(JSON.parse(e.init?.stdin ?? 'null'))
    return RAN
  })
  const clock = mock.clock(on)

  await $.tool.check(asked('t1'))
  await clock.advance(1000)
  // Esc: the dialog closes with the turn.
  await $.turn.complete({ ...END, isAborted: true, reason: 'aborted', turnId: 'turn-1' })
  await clock.advance(10_000)

  expect(sent).toEqual([{ ev: 'idle' }])
})
