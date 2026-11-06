// ccs-traces where an organization manages Claude Code: its built-in guard, `sec-default`, sits
// outermost and sends some events on past every mod the user installed (code.claude.com/docs/en/
// plugins/mods/admin, "Know which hooks of a user's mod don't run"). GUARD stands in for it.
import type { On, SessionMessage } from 'claude-code'
import type { Engine, Plugin } from 'claude-code/testing'
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

const VIEW = { columns: 100, rows: 40, isFullscreen: true }

/** A stored conversation of two calls in one run, `<id>1` and `<id>2`. */
const stored = (id: string): SessionMessage[] => [
  { role: 'user', text: 'go', toolUses: [] },
  { role: 'assistant', text: '', toolUses: [{ tool_use_id: `${id}1`, tool: 'Read', input: {} }] },
  { role: 'user', text: '', toolUses: [], toolResults: [{ tool_use_id: `${id}1`, text: 'x', isError: false }] },
  { role: 'assistant', text: '', toolUses: [{ tool_use_id: `${id}2`, tool: 'Grep', input: {} }] },
  { role: 'user', text: '', toolUses: [], toolResults: [{ tool_use_id: `${id}2`, text: 'x', isError: false }] },
]

/** The engine beneath the mod, with the conversation `messages()` gives. */
function engine(on: On, messages: () => SessionMessage[]) {
  on('session.messages', () => ({ value: messages() }))
  on('session.start', (_, e) => ({ cwd: e.cwd }))
  on('session.end', (_, e) => ({ sessionId: e.sessionId }))
  on('classic.SessionStart', () => ({}))
  on('ui.render', { component: 'ToolUse' }, ($, e) => {
    const { Text } = $.ui.resolve(e)
    return <Text>engine</Text>
  })
  return mock.clock(on)
}

function row($: Engine, id: string, tool: string) {
  return $.ui.mount({
    plugin: 'ccs-traces',
    surface: 'terminal',
    component: 'ToolUse',
    props: { tool_use_id: id, tool, input: {}, isRunning: false, isErrored: false, isInterrupted: false },
    requestId: id,
    viewport: VIEW,
  })
}

const END = { sessionId: 's1', resume: { id: 's1' } }

test('the classic path misses a resume under the guard', { plugins: [GUARD] }, async ($, on) => {
  let messages = stored('a')
  const clock = engine(on, () => messages)
  await $.session.start({ cwd: '/p', surface: 'terminal', isInteractive: true })
  await clock.settle()

  messages = stored('b')
  await $.session.end({ ...END, reason: 'resume' })
  await $.classic.SessionStart({ source: 'resume' })
  await clock.settle()
  const ui = await row($, 'b1', 'Read')
  await clock.settle()

  // Not folded: the resumed calls are in no run.
  expect(await ui.find({ key: 'run' })).toBeUndefined()
})

test(
  'the guarded path folds a resumed conversation from its first row',
  { plugins: [GUARD], options: { signals: 'guarded' } },
  async ($, on) => {
    let messages = stored('a')
    const clock = engine(on, () => messages)
    await $.session.start({ cwd: '/p', surface: 'terminal', isInteractive: true })
    await clock.settle()

    messages = stored('b')
    await $.session.end({ ...END, reason: 'resume' })
    await $.classic.SessionStart({ source: 'resume' })
    // Its rows draw before any is appended.
    const ui = await row($, 'b1', 'Read')
    await clock.settle()

    expect((await ui.find({ key: 'run' }))?.text).toBe('● 2 tool calls · Read 1 · Grep 1')
  },
)

/** Claude calls Bash as `id`. */
function call($: Engine, id: string) {
  return $.session.append({
    message: { type: 'assistant', role: 'assistant', content: [{ type: 'tool_use', id, name: 'Bash', input: {} }] },
    door: 'response',
    origin: { kind: 'model', model: 'm' },
    uuid: `row-${id}`,
  })
}

test(
  'the guarded path starts afresh after /clear, so new calls join no run from before',
  { plugins: [GUARD], options: { signals: 'guarded' } },
  async ($, on) => {
    // The run of `a1` and `a2` is still open: no text came after it.
    let messages = stored('a')
    const clock = engine(on, () => messages)
    await $.session.start({ cwd: '/p', surface: 'terminal', isInteractive: true })
    await clock.settle()

    messages = []
    await $.session.end({ ...END, reason: 'clear' })
    await call($, 'c1')
    await call($, 'c2')
    await clock.settle()

    expect((await (await row($, 'c1', 'Bash')).find({ key: 'run' }))?.text).toBe('● 2 tool calls · Bash 2 · 2 running')
  },
)
