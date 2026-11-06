// What this mod sends the ccs that runs the session, which reads it in
// `crates/ccs-claude/src/bridge.rs`.
import type { EngineInterface } from 'claude-code'

/** One report; `ev` names it. */
export type Report =
  | { ev: 'ready'; session_id: string; root: string }
  | { ev: 'submitted' | 'working' | 'permission' | 'approved' | 'denied' | 'asleep' | 'idle' }
  | { ev: 'title' | 'guess'; title: string }
  | { ev: 'root'; root: string }

/** The reports leave one at a time, so ccs reads them in order. */
let sent: Promise<unknown> = Promise.resolve()

export function report($: EngineInterface, event: Report) {
  sent = sent.then(() => send($, event)).catch(() => undefined)
}

async function send($: EngineInterface, event: Report) {
  const exe = await $.env.get('CCS_EXE')
  const socket = await $.env.get('CCS_SOCKET')
  const session = await $.env.get('CCS_SESSION')
  if (exe === undefined || socket === undefined || session === undefined) return
  await $.process.run([exe, 'hook', socket, session], { stdin: JSON.stringify(event) })
}
