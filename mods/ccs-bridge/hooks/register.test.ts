import type { On } from 'claude-code'
import { expect, mock, test } from 'claude-code/testing'

/** What `ccs hook` answers: nothing. */
const RAN = {
  value: { exitCode: 0, stdout: '', stderr: '', isStdoutTruncated: false, isStderrTruncated: false },
}

/** The engine beneath the mod, for the events these tests raise. */
function engine(on: On) {
  on('classic.PermissionRequest', () => ({}))
  on('turn.complete', () => ({ text: '' }))
}
/** What ccs gives the Claude it starts. */
const ENV = { CCS_EXE: '/opt/ccs', CCS_SOCKET: '/run/ccs.sock', CCS_SESSION: '3' }

test('a dialog, then an interrupt, reports permission then idle to ccs', async ($, on) => {
  engine(on)
  mock.env(on, ENV)
  const sent: unknown[] = []
  on('process.run', ($, e) => {
    sent.push({ argv: e.argv, report: JSON.parse(e.init?.stdin ?? 'null') })
    return RAN
  })
  const clock = mock.clock(on)

  await $.classic.PermissionRequest({
    tool_name: 'Write',
    tool_input: { file_path: 'a.txt', content: 'a' },
    permission_mode: 'default',
  })
  await $.turn.complete({
    answer: '',
    durationMs: 10,
    isAborted: true,
    turnId: 'turn-1',
    reason: 'aborted',
  })
  // The reports leave one after the other.
  await clock.settle()

  const argv = ['/opt/ccs', 'hook', '/run/ccs.sock', '3']
  expect(sent).toEqual([
    { argv, report: { ev: 'permission' } },
    { argv, report: { ev: 'idle' } },
  ])
})

test('without ccs it reports nothing', async ($, on) => {
  engine(on)
  mock.env(on, {})
  let runs = 0
  on('process.run', () => {
    runs += 1
    return RAN
  })
  const clock = mock.clock(on)
  await $.classic.PermissionRequest({
    tool_name: 'Write',
    tool_input: { file_path: 'a.txt', content: 'a' },
    permission_mode: 'default',
  })
  await clock.settle()
  expect(runs).toBe(0)
})

test('ready says where the session works, and a tool call that moves it reports the move', async ($, on) => {
  let root = '/work/.claude/worktrees/ccs-1'
  on('session.start', ($, e) => ({ cwd: e.cwd }))
  on('session.id', () => ({ value: '5c1c' }))
  on('session.root', () => ({ value: root }))
  on('tool.call', () => ({ result: 'done' }))
  mock.env(on, ENV)
  const sent: unknown[] = []
  on('process.run', ($, e) => {
    sent.push(JSON.parse(e.init?.stdin ?? 'null'))
    return RAN
  })
  const clock = mock.clock(on)

  await $.session.start({ cwd: root, surface: 'terminal', isInteractive: true })
  // A call that leaves the session where it is says nothing.
  await $.tool.call({ tool: 'Bash', tool_use_id: 't1', command: 'ls' })
  root = '/work'
  await $.tool.call({ tool: 'ExitWorktree', tool_use_id: 't2', action: 'keep' })
  await $.tool.call({ tool: 'Bash', tool_use_id: 't3', command: 'ls' })
  await clock.settle()

  expect(sent).toEqual([
    { ev: 'ready', session_id: '5c1c', root: '/work/.claude/worktrees/ccs-1' },
    { ev: 'root', root: '/work' },
  ])
})
