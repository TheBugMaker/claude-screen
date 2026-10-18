// Draws each run of tool calls in the transcript (the calls Claude makes between two of its texts)
// as one line, `● 5 tool calls · Read 3 · Bash 2`. A click on the line lists the calls, a click
// on a call shows its whole input and output, and a click on them folds it again. It draws only
// where clicks reach it: the fullscreen terminal.
import { atom, memberOf, read, update } from 'claude-code'
import type { Elements, EngineInterface, RenderNode, Register, SessionMessage } from 'claude-code'

import type { Call } from '../types'

const RUN_OF = { plugin: 'ccs-traces', key: 'runOf' } as const
const CALLS = { plugin: 'ccs-traces', key: 'calls' } as const
const runOf = atom(RUN_OF, '')
const calls = atom(CALLS, [])
const isOpen = atom({ plugin: 'ccs-traces', key: 'isOpen' } as const, false)
const isShown = atom({ plugin: 'ccs-traces', key: 'isShown' } as const, false)

/** What one transcript row says about the runs. */
type Step =
  /** Words you read: the run ends. */
  | { kind: 'text' }
  | { kind: 'call'; id: string; tool: string }
  | { kind: 'end'; id: string; isError: boolean }

/** Every run, by its first call's id, and the one the next call joins (`''`: none). */
type Index = { open: string; runs: Map<string, Call[]>; runOf: Map<string, string> }

/** What `feed` changed: the runs, and the calls that joined one. */
type Change = { runs: Set<string>; joined: string[] }

const fresh = (): Index => ({ open: '', runs: new Map(), runOf: new Map() })

/** The main conversation's runs. A reload loses it, and `session.start` builds it again. */
let index = fresh()
/** The index's work, one piece at a time, so the rows are fed in the order they came. */
let work: Promise<unknown> = Promise.resolve()

function queue(job: () => Promise<void>) {
  work = work.then(job).catch(() => undefined)
}

function feed(into: Index, steps: Step[]): Change {
  const change: Change = { runs: new Set(), joined: [] }
  for (const step of steps) {
    if (step.kind === 'text') {
      into.open = ''
    } else if (step.kind === 'call') {
      if (into.runOf.has(step.id)) continue
      if (into.open === '') {
        into.open = step.id
        into.runs.set(step.id, [])
      }
      into.runs.get(into.open)?.push({ id: step.id, tool: step.tool, status: 'running' })
      into.runOf.set(step.id, into.open)
      change.runs.add(into.open)
      change.joined.push(step.id)
    } else {
      const run = into.runOf.get(step.id)
      const call = run === undefined ? undefined : into.runs.get(run)?.find(c => c.id === step.id)
      if (run === undefined || call === undefined) continue
      call.status = step.isError ? 'errored' : 'done'
      change.runs.add(run)
    }
  }
  return change
}

/** Writes what changed, the runs first: a row wakes on its `runOf`, and then reads its run. */
async function write($: EngineInterface, from: Index, change: Change) {
  for (const run of change.runs) {
    await $.state.set(memberOf(CALLS, { requestId: run }), from.runs.get(run) ?? [])
  }
  for (const id of change.joined) {
    await $.state.set(memberOf(RUN_OF, { requestId: id }), from.runOf.get(id) ?? '')
  }
}

const said = (text: unknown): Step[] =>
  typeof text === 'string' && text.trim() !== '' ? [{ kind: 'text' }] : []

/** The steps of a row as it is appended: an assistant's blocks, or a user's results or words. */
function appended(message: {
  role?: 'user' | 'assistant'
  isMeta?: true
  content: readonly { type: string; [field: string]: unknown }[]
}): Step[] {
  if (message.role === 'assistant') {
    return message.content.flatMap((block): Step[] =>
      block.type === 'tool_use'
        ? [{ kind: 'call', id: String(block.id), tool: String(block.name) }]
        : block.type === 'text'
          ? said(block.text)
          : [],
    )
  }
  // A reminder or a nudge is not shown, so it splits nothing.
  if (message.role !== 'user' || message.isMeta) return []
  const results = message.content.filter(block => block.type === 'tool_result')
  if (results.length > 0) {
    return results.map(block => ({
      kind: 'end',
      id: String(block.tool_use_id),
      isError: block.is_error === true,
    }))
  }
  return message.content.flatMap(block => (block.type === 'text' ? said(block.text) : []))
}

/** The steps of a row of the stored conversation, which leaves the reminders out. */
function stored(message: SessionMessage): Step[] {
  if (message.role === 'assistant') {
    return [
      ...said(message.text),
      ...message.toolUses.map((use): Step => ({ kind: 'call', id: use.tool_use_id, tool: use.tool })),
    ]
  }
  const results = message.toolResults ?? []
  if (results.length > 0) {
    return results.map(result => ({ kind: 'end', id: result.tool_use_id, isError: result.isError }))
  }
  return said(message.text)
}

/** Builds the index from the conversation so far: at start, on a resume, after `/clear`. */
function rebuild($: EngineInterface) {
  queue(async () => {
    const built = fresh()
    const change = feed(built, (await $.session.messages()).flatMap(stored))
    index = built
    await write($, built, change)
  })
}

/** The rows that change runs: Claude's blocks, results, and what the person sends. */
const DOORS = new Set(['response', 'tool-result', 'prompt', 'command', 'delivery'])

function headline(list: readonly Call[]): string {
  const counts = new Map<string, number>()
  for (const call of list) counts.set(call.tool, (counts.get(call.tool) ?? 0) + 1)
  const parts = [`${list.length} tool calls`, ...[...counts].map(([tool, n]) => `${tool} ${n}`)]
  const running = list.filter(call => call.status === 'running').length
  const errored = list.filter(call => call.status === 'errored').length
  if (running > 0) parts.push(`${running} running`)
  if (errored > 0) parts.push(`${errored} errored`)
  return parts.join(' · ')
}

/** `Tool(first line of its first text argument)`, as Claude names a call, at most `width` long. */
function label(tool: string, input: unknown, width: number): string {
  const values = typeof input === 'object' && input !== null ? Object.values(input) : []
  const first = values.find((value): value is string => typeof value === 'string')
  const name = first === undefined ? tool : `${tool}(${first.split('\n')[0]})`
  return name.length > width ? `${name.slice(0, Math.max(0, width - 1))}…` : name
}

/** The elements, and what a click on the drawn text does. */
type Draw = Pick<Elements['terminal'], 'Box' | 'Text' | 'Button'> & { fold: () => void }

const isLine = (value: unknown) =>
  (typeof value === 'string' && !value.includes('\n')) ||
  typeof value === 'number' ||
  typeof value === 'boolean' ||
  value === null

/** A line of a call's trace, named `key`: a click on it folds the call. */
function line(draw: Draw, key: string, ...children: RenderNode[]): RenderNode {
  const { Button, fold } = draw
  return (
    <Button key={key} plain onPress={fold}>
      {children}
    </Button>
  )
}

/** `value` whole, at `at`: text as it is, an object field by field, anything else as JSON. */
function whole(value: unknown, draw: Draw, at: string): RenderNode {
  const { Box, Text } = draw
  if (typeof value === 'string') return line(draw, at, value)
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    return line(draw, at, JSON.stringify(value, null, 2) ?? String(value))
  }
  // As JSON has it, a field that is undefined is not there.
  const fields = Object.entries(value).filter(([, field]) => field !== undefined)
  return (
    <Box flexDirection="column">
      {fields.map(([key, field]) =>
        isLine(field) ? (
          line(draw, `${at}.${key}`, <Text dimColor>{key}: </Text>, String(field))
        ) : (
          <Box flexDirection="column">
            {line(draw, `${at}.${key}:`, <Text dimColor>{key}:</Text>)}
            <Box marginLeft={2}>{whole(field, draw, `${at}.${key}`)}</Box>
          </Box>
        ),
      )}
    </Box>
  )
}

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    const result = await next(e)
    rebuild($)
    return result
  })
  // `/resume`, `/clear` and a compaction change the conversation without a `session.start`.
  on('classic.SessionStart', async ($, e, next) => {
    const result = await next(e)
    if (e.source !== 'startup') rebuild($)
    return result
  })
  on('session.append', ($, e, next) => {
    if (e.agentId === undefined && DOORS.has(e.door)) {
      const steps = appended(e.message)
      if (steps.length > 0) queue(() => write($, index, feed(index, steps)))
    }
    return next(e)
  }).catch(($, e, next) => next(e))
  // An Esc can end calls without a result.
  on('turn.complete', ($, e, next) => {
    if (e.agentId === undefined) {
      queue(() => {
        const running = [...index.runs.values()].flat().filter(call => call.status === 'running')
        const ends = running.map((call): Step => ({ kind: 'end', id: call.id, isError: e.isAborted }))
        return write($, index, feed(index, ends))
      })
    }
    return next(e)
  })

  on('ui.render', { component: 'ToolGroup' }, ($, e, next) =>
    next(e.viewport?.isFullscreen ? { ...e, props: { ...e.props, isExpanded: true } } : e),
  )
  // The call's output is in its row's trace.
  on('ui.render', { component: 'ToolResult' }, async ($, e, next) => {
    if (!e.viewport?.isFullscreen || (await read($, memberOf(runOf, e))) === '') return next(e)
    const { Box } = $.ui.resolve(e)
    return <Box />
  })
  on('ui.render', { component: 'ToolUse' }, async ($, e, next) => {
    // ccs-bridge, beneath, sees an approval in the row's `isRunning`.
    const engine = await next(e)
    if (!e.viewport?.isFullscreen) return engine
    const run = await read($, memberOf(runOf, e))
    if (run === '') return engine
    const isFirst = run === e.requestId
    const list = isFirst ? await read($, memberOf(calls, { requestId: run })) : []
    // A run of one call is drawn as the call.
    const isAlone = isFirst && list.length <= 1
    const open = isAlone || (await read($, memberOf(isOpen, { requestId: run })))
    const { Box, Text, Button } = $.ui.resolve(e)
    if (!isFirst && !open) return <Box />
    const shown = open && (await read($, memberOf(isShown, e)))
    const toggle = () => update($, memberOf(isShown, e), v => !v)
    const draw = { Box, Text, Button, fold: toggle }
    const { isRunning, isErrored, isInterrupted, input, output } = e.props
    const bullet = (status: { isErrored: boolean; isRunning: boolean }) => (
      <Text color={status.isErrored ? 'error' : status.isRunning ? 'inactive' : 'success'}>● </Text>
    )
    const fails = isErrored || isInterrupted
    return (
      <Box flexDirection="column" marginTop={isFirst ? 1 : 0}>
        {isFirst && !isAlone && (
          <Button key="run" plain onPress={() => update($, memberOf(isOpen, { requestId: run }), v => !v)}>
            {bullet({
              isErrored: list.some(call => call.status === 'errored'),
              isRunning: list.some(call => call.status === 'running'),
            })}
            {headline(list)}
          </Button>
        )}
        {open && (
          <Button key="call" plain onPress={toggle}>
            {isAlone ? bullet({ isErrored: fails, isRunning }) : shown ? '  ▾ ' : '  ▸ '}
            {label(e.props.tool, input, e.viewport.columns - 8)}
            {isRunning && <Text dimColor> …</Text>}
            {fails && <Text color="error"> ✗</Text>}
          </Button>
        )}
        {shown && (
          <Box flexDirection="column" marginLeft={4}>
            {line(draw, 'input:', <Text dimColor>input</Text>)}
            <Box marginLeft={2}>{whole(input, draw, 'input')}</Box>
            {line(draw, 'output:', <Text dimColor>output</Text>)}
            <Box marginLeft={2}>
              {output === undefined
                ? line(draw, 'output', <Text dimColor>(running)</Text>)
                : whole(output, draw, 'output')}
            </Box>
          </Box>
        )}
      </Box>
    )
  })
}
