import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register } from 'claude-code'

import { flatten } from './tree'
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

async function refresh($: EngineInterface) {
  try {
    const listed = await tt($, ['list', '--json'])
    if (listed.exitCode !== 0) throw new Error(listed.stderr.trim())
    const tree = JSON.parse(listed.stdout) as TreeJson
    await update($, rows, () => flatten(tree, sessionId))
    await update($, error, () => '')
  } catch (err) {
    await update($, error, () => String(err))
  }
  $.ui.invalidate('ui.render')
}

async function finish($: EngineInterface, args: string[]) {
  const ran = await tt($, [...args, '--session', sessionId])
  await $.ui.close({ id: PANE })
  $.ui.toast(ran.exitCode === 0 ? ran.stdout.trim() : `tt: ${ran.stderr.trim()}`)
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
    return { text: 'Pick a todo in the pane (Esc to cancel).' }
  })

  on('ui.render', { component: 'Pane', requestId: PANE }, async ($, e) => {
    const elements = $.ui.resolve(e)
    const { Box, Text, Button } = elements
    const Input = 'Input' in elements ? elements.Input : undefined
    const list = await read($, rows)
    const failed = await read($, error)
    const room = Math.max(3, (e.viewport?.rows ?? 24) - 6)

    return (
      <Box key="claim" flexDirection="column">
        {failed !== '' && <Text color="red">{failed}</Text>}
        {failed === '' && list.length === 0 && <Text dimColor>No open todos yet. Add one below.</Text>}
        {list.slice(0, room).map((row, i) => (
          <Button
            key={`todo-${row.id}`}
            plain
            autoFocus={i === 0 ? true : undefined}
            label={`${'  '.repeat(row.depth)}${row.isClaimed ? '● ' : '○ '}${row.title}`}
            onPress={() => void finish($, row.isClaimed ? ['unclaim', String(row.id)] : ['claim', String(row.id)])}
          />
        ))}
        {list.length > room && <Text dimColor>…{list.length - room} more; nest or finish some in tt</Text>}
        {Input && (
          <Input
            key="new"
            placeholder="or type a new todo and press Enter"
            submitLabel="add + claim"
            onSubmit={(value: string) => {
              if (value.trim() !== '') void finish($, ['add', value.trim(), '--claim'])
            }}
          />
        )}
        <Text dimColor>⏎ claim · ● already claimed here (⏎ detaches) · Esc cancel</Text>
      </Box>
    )
  })
}
