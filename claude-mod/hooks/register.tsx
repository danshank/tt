import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register } from 'claude-code'

import type { PickAction } from '../types'
import { claimNote, flatten } from './tree'
import type { TreeJson } from './tree'

const PANE = 'tt-claim'
const rows = atom({ plugin: 'tt-claim', key: 'rows' } as const, [])
const error = atom({ plugin: 'tt-claim', key: 'error' } as const, '')

const TT = ['tt', '/Users/dan/Software/personal/tt/target/debug/tt']

async function tt($: EngineInterface, args: string[]) {
  const cwd = await $.session.cwd()
  for (const bin of TT) {
    try {
      return await $.process.run([bin, ...args], { cwd })
    } catch {
      // try the next location
    }
  }
  throw new Error('tt not found on PATH or in target/debug')
}

async function loadTree($: EngineInterface): Promise<TreeJson> {
  const listed = await tt($, ['list', '--json'])
  if (listed.exitCode !== 0) throw new Error(listed.stderr.trim())
  return JSON.parse(listed.stdout) as TreeJson
}

async function refresh($: EngineInterface) {
  try {
    const tree = await loadTree($)
    await update($, rows, () => flatten(tree, sessionId))
    await update($, error, () => '')
  } catch (err) {
    await update($, error, () => String(err))
  }
  $.ui.invalidate('ui.render')
}

function args(a: PickAction): string[] {
  switch (a.kind) {
    case 'claim':
    case 'unclaim':
      return [a.kind, String(a.id)]
    case 'add':
      return [
        'add',
        a.title,
        '--claim',
        ...(a.after === undefined ? [] : ['--after', String(a.after)]),
        ...(a.parent === undefined ? [] : ['--parent', String(a.parent)]),
      ]
  }
}

/** Run the picked action, close the pane, and tell both the person and the model. */
async function finish($: EngineInterface, action: PickAction) {
  const ran = await tt($, [...args(action), '--session', sessionId])
  await $.ui.close({ id: PANE })
  if (ran.exitCode !== 0) {
    $.ui.toast(`tt: ${ran.stderr.trim()}`)
    return
  }
  $.ui.toast(ran.stdout.trim())
  const id = action.kind === 'add' ? Number(/#(\d+)/.exec(ran.stdout)?.[1]) : action.id
  const row = flatten(await loadTree($), sessionId).find(r => r.id === id)
  const note = row
    ? claimNote(row, action.kind === 'unclaim' ? 'Detached from' : 'Claimed')
    : `${action.kind === 'unclaim' ? 'Detached from' : 'Claimed'} tt todo #${id}`
  await $.session.append({ message: { type: 'user', content: [{ type: 'text', text: note }] } })
}

let sessionId = ''

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    sessionId = await $.session.id()
    await $.command.register({ name: 'claim', description: 'Pick a tt todo for this session to take on' })
    return next(e)
  })

  on('command.run', { command: 'claim' }, async $ => {
    sessionId = sessionId || (await $.session.id())
    await refresh($)
    await $.ui.open({ id: PANE, title: 'Claim a todo', focus: true, closeOnEscape: true })
    return { text: 'Click the pane, then pick a todo (Esc to cancel).' }
  })

  on('ui.message', async ($, e, next) => {
    if (e.requestId !== PANE) return next(e)
    await finish($, e.data as PickAction)
    return {}
  })

  on('ui.render', { component: 'Pane', requestId: PANE }, async ($, e) => {
    const elements = $.ui.resolve(e)
    if (!('Client' in elements)) return <elements.Text>Open /claim in the terminal or desktop app.</elements.Text>
    const { Client } = elements
    const list = await read($, rows)
    const failed = await read($, error)
    const room = Math.max(5, (e.viewport?.rows ?? 24) - 4)
    return <Client key="picker" module="./picker.tsx" props={{ rows: list, error: failed }} height={Math.min(list.length + 3, room)} />
  })
}
