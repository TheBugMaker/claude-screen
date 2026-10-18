import type { On, RenderPropsOf, SessionMessage } from 'claude-code'
import { expect, mock, test } from 'claude-code/testing'
import type { Engine } from 'claude-code/testing'

const VIEW = { columns: 100, rows: 40, isFullscreen: true }

/** The engine beneath the mod: it draws a tool row as `engine`. */
function engine(on: On, drawn: string[] = []) {
  on('turn.complete', () => ({ text: '' }))
  on('ui.render', { component: 'ToolUse' }, ($, e) => {
    drawn.push(e.requestId)
    const { Text } = $.ui.resolve(e)
    return <Text>engine</Text>
  })
  on('ui.render', { component: 'ToolResult' }, ($, e) => {
    const { Text } = $.ui.resolve(e)
    return <Text>result</Text>
  })
  return mock.clock(on)
}

let rows = 0

/** Claude calls `tool` as `id`. */
function call($: Engine, id: string, tool: string, agentId?: string) {
  return $.session.append({
    message: {
      type: 'assistant',
      role: 'assistant',
      content: [{ type: 'tool_use', id, name: tool, input: {} }],
    },
    door: 'response',
    origin: { kind: 'model', model: 'm' },
    uuid: `row-${(rows += 1)}`,
    agentId,
  })
}

/** Claude writes `text`. */
function say($: Engine, text: string) {
  return $.session.append({
    message: { type: 'assistant', role: 'assistant', content: [{ type: 'text', text }] },
    door: 'response',
    origin: { kind: 'model', model: 'm' },
    uuid: `row-${(rows += 1)}`,
  })
}

/** The result of call `id`. */
function result($: Engine, id: string, isError = false) {
  return $.session.append({
    message: {
      type: 'user',
      role: 'user',
      content: [{ type: 'tool_result', tool_use_id: id, content: 'out', is_error: isError }],
    },
    door: 'tool-result',
    origin: { kind: 'tool', tool: 'Bash' },
    uuid: `row-${(rows += 1)}`,
  })
}

/** A user row of `text`, the person's own or a hidden one. */
function user($: Engine, text: string, isMeta?: true) {
  return $.session.append({
    message: { type: 'user', role: 'user', isMeta, content: [{ type: 'text', text }] },
    door: isMeta ? 'delivery' : 'prompt',
    origin: { kind: 'composer' },
    uuid: `row-${(rows += 1)}`,
  })
}

function props(id: string, tool: string, input: unknown = {}): RenderPropsOf['ToolUse'] {
  return {
    tool_use_id: id,
    tool,
    input,
    isRunning: false,
    isErrored: false,
    isInterrupted: false,
    output: { stdout: 'line 1\nline 2', exitCode: 0, gitOperation: undefined },
  }
}

function row($: Engine, id: string, tool: string, input?: unknown, viewport = VIEW) {
  return $.ui.mount({
    plugin: 'ccs-traces',
    surface: 'terminal',
    component: 'ToolUse',
    props: props(id, tool, input),
    requestId: id,
    viewport,
  })
}

/** Everything a drawing shows, as text. */
async function shown(ui: { findAll: (q: object) => Promise<{ text: string }[]> }) {
  const texts = await ui.findAll({ type: 'Text' })
  const buttons = await ui.findAll({ type: 'Button' })
  return [...buttons, ...texts].map(found => found.text).join('\n')
}

test('a run of calls is one line on its first row, and its other rows draw nothing', async ($, on) => {
  const clock = engine(on)
  await call($, 'a', 'Read')
  await result($, 'a')
  await call($, 'b', 'Edit')
  await result($, 'b')
  await call($, 'c', 'Bash')
  await clock.settle()

  const first = await row($, 'a', 'Read', { file_path: 'a.txt' })
  expect((await first.find({ key: 'run' }))?.text).toBe('● 3 tool calls · Read 1 · Edit 1 · Bash 1 · 1 running')
  expect(await first.find({ key: 'call' })).toBeUndefined()
  expect(await shown(await row($, 'b', 'Edit'))).toBe('')
  expect(await shown(await row($, 'c', 'Bash'))).toBe('')
  const output = await $.ui.mount({
    plugin: 'ccs-traces',
    surface: 'terminal',
    component: 'ToolResult',
    props: { tool_use_id: 'a', tool: 'Read', output: 'out', isErrored: false },
    requestId: 'a',
    viewport: VIEW,
  })
  expect(await shown(output)).toBe('')
})

test('text between calls splits a run, and blank text, hidden rows and results do not', async ($, on) => {
  const clock = engine(on)
  await call($, 'a', 'Read')
  await result($, 'a')
  await say($, '  \n')
  await user($, 'a reminder', true)
  await call($, 'b', 'Read')
  await say($, 'Now the tests.')
  await call($, 'c', 'Bash')
  await user($, 'and then?')
  await call($, 'd', 'Bash')
  await clock.settle()

  expect((await (await row($, 'a', 'Read')).find({ key: 'run' }))?.text).toMatch(/^● 2 tool calls/)
  expect(await shown(await row($, 'b', 'Read'))).toBe('')
  // Alone in their runs, c and d are drawn as calls.
  expect((await (await row($, 'c', 'Bash', { command: 'make test' })).find({ key: 'call' }))?.text).toBe(
    '● Bash(make test)',
  )
  expect((await (await row($, 'd', 'Bash')).find({ key: 'call' }))?.text).toBe('● Bash')
})

test('a click lists the calls, and a click on a call shows its whole input and output', async ($, on) => {
  const clock = engine(on)
  await call($, 'a', 'Read')
  await call($, 'b', 'Bash')
  await clock.settle()
  const first = await row($, 'a', 'Read', { file_path: 'a.txt' })
  const second = await row($, 'b', 'Bash', { command: 'git status\n--short', description: 'Status' })

  await first.press({ key: 'run' })
  expect((await first.find({ key: 'call' }))?.text).toBe('  ▸ Read(a.txt)')
  expect((await second.find({ key: 'call' }))?.text).toBe('  ▸ Bash(git status)')

  await second.press({ key: 'call' })
  const trace = await shown(second)
  expect(trace).toContain('  ▾ Bash(git status)')
  expect(trace).toContain('git status\n--short')
  expect(trace).toContain('description: Status')
  expect(trace).toContain('line 1\nline 2')
  expect(trace).toContain('exitCode: 0')
  expect(trace).not.toContain('gitOperation')

  await second.press({ key: 'call' })
  expect(await shown(second)).not.toContain('line 1')
  await first.press({ key: 'run' })
  expect(await shown(second)).toBe('')
})

test('an errored call is counted, and the end of a turn ends the calls left running', async ($, on) => {
  const clock = engine(on)
  await call($, 'a', 'Read')
  await result($, 'a', true)
  await call($, 'b', 'Bash')
  await clock.settle()
  const first = await row($, 'a', 'Read')
  expect((await first.find({ key: 'run' }))?.text).toBe('● 2 tool calls · Read 1 · Bash 1 · 1 running · 1 errored')

  await $.turn.complete({ answer: '', durationMs: 10, isAborted: true, turnId: 't', reason: 'aborted' })
  await clock.settle()
  expect((await first.find({ key: 'run' }))?.text).toBe('● 2 tool calls · Read 1 · Bash 1 · 2 errored')
})

test('unknown and subagent calls, and every row off the fullscreen terminal, draw as the engine', async ($, on) => {
  const clock = engine(on)
  await call($, 'a', 'Read')
  await call($, 'b', 'Read')
  await call($, 's', 'Bash', 'agent-1')
  await clock.settle()

  expect(await shown(await row($, 'x', 'Read'))).toBe('engine')
  expect(await shown(await row($, 's', 'Bash'))).toBe('engine')
  const main = { columns: 100, rows: 40, isFullscreen: false }
  expect(await shown(await row($, 'b', 'Read', {}, main))).toBe('engine')
})

test('every row still reaches the hooks beneath, as ccs-bridge needs', async ($, on) => {
  const drawn: string[] = []
  const clock = engine(on, drawn)
  await call($, 'a', 'Read')
  await call($, 'b', 'Read')
  await clock.settle()
  await row($, 'a', 'Read')
  await row($, 'b', 'Read')
  expect(drawn).toEqual(['a', 'b'])
})

test('a resumed conversation is folded, from session.start and from /resume', async ($, on) => {
  const clock = engine(on)
  const stored = (id: string): SessionMessage[] => [
    { role: 'user', text: 'go', toolUses: [] },
    { role: 'assistant', text: '', toolUses: [{ tool_use_id: `${id}1`, tool: 'Read', input: {} }] },
    { role: 'user', text: '', toolUses: [], toolResults: [{ tool_use_id: `${id}1`, text: 'x', isError: false }] },
    { role: 'assistant', text: '', toolUses: [{ tool_use_id: `${id}2`, tool: 'Grep', input: {} }] },
    { role: 'user', text: '', toolUses: [], toolResults: [{ tool_use_id: `${id}2`, text: 'x', isError: false }] },
  ]
  let messages = stored('a')
  on('session.messages', () => ({ value: messages }))
  on('session.start', (_, e) => ({ cwd: e.cwd }))
  on('classic.SessionStart', () => ({}))

  await $.session.start({ cwd: '/p', surface: 'terminal', isInteractive: true })
  await clock.settle()
  expect((await (await row($, 'a1', 'Read')).find({ key: 'run' }))?.text).toBe('● 2 tool calls · Read 1 · Grep 1')

  messages = stored('b')
  await $.classic.SessionStart({ source: 'resume' })
  await clock.settle()
  expect((await (await row($, 'b1', 'Read')).find({ key: 'run' }))?.text).toBe('● 2 tool calls · Read 1 · Grep 1')
  expect(await shown(await row($, 'b2', 'Grep'))).toBe('')
})

test('a group of calls is unfolded into its rows', async ($, on) => {
  engine(on)
  let expanded: boolean | undefined
  on('ui.render', { component: 'ToolGroup' }, ($, e) => {
    expanded = e.props.isExpanded
    const { Box } = $.ui.resolve(e)
    return <Box />
  })
  await $.ui.mount({
    plugin: 'ccs-traces',
    surface: 'terminal',
    component: 'ToolGroup',
    props: { calls: [], isActive: false, isExpanded: false },
    viewport: VIEW,
  })
  expect(expanded).toBe(true)
})
