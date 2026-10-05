import type { ClientKeyEvent, ClientModule } from 'claude-code'

import type { PickAction, PickRow } from '../types'
import { selectable, visible } from './tree'
import type { ShownRow } from './tree'

type Props = { rows: PickRow[]; error: string }
type Mode = 'list' | 'search' | 'sibling' | 'child'
type State = { cursor: number | null; top: number; collapsed: number[]; mode: Mode; query: string; draft: string }

const CHROME = 2

const typed = (k: ClientKeyEvent) => k.key.length === 1 && !k.ctrl && !k.meta

/** Next state for one key, plus what to post to the hooks module, if anything. */
export function press(s: State, rows: PickRow[], k: ClientKeyEvent): [State, PickAction?] {
  const shown = visible(rows, s.collapsed, s.query)
  const cur = shown.find(r => r.id === s.cursor)
  const firstMatch = (q: string) => visible(rows, s.collapsed, q).find(r => r.isMatch)?.id ?? null

  if (s.mode === 'sibling' || s.mode === 'child') {
    if (k.key === 'return') {
      const title = s.draft.trim()
      if (title === '') return [{ ...s, mode: 'list' }]
      const where = cur === undefined ? {} : s.mode === 'child' ? { parent: cur.id } : { after: cur.id }
      return [{ ...s, mode: 'list', draft: '' }, { kind: 'add', title, ...where }]
    }
    if (k.key === 'backspace') return [s.draft === '' ? { ...s, mode: 'list' } : { ...s, draft: s.draft.slice(0, -1) }]
    if (typed(k)) return [{ ...s, draft: s.draft + k.key }]
    return [s]
  }

  if (s.mode === 'search') {
    if (k.key === 'return') return [{ ...s, mode: 'list' }]
    if (k.key === 'backspace') {
      if (s.query === '') return [{ ...s, mode: 'list' }]
      const query = s.query.slice(0, -1)
      return [{ ...s, query, cursor: query === '' ? s.cursor : firstMatch(query) }]
    }
    if (typed(k)) {
      const query = s.query + k.key
      return [{ ...s, query, cursor: firstMatch(query) }]
    }
  }

  const pickable = shown.filter(selectable)
  const at = pickable.findIndex(r => r.id === s.cursor)
  switch (k.key) {
    case 'up':
      return [{ ...s, cursor: pickable[Math.max(0, at - 1)]?.id ?? null }]
    case 'down':
      return [{ ...s, cursor: pickable[Math.min(pickable.length - 1, at + 1)]?.id ?? null }]
    case 'left':
      if (cur && cur.hasChildren && !cur.isCollapsed && s.query === '') return [{ ...s, collapsed: [...s.collapsed, cur.id] }]
      if (cur?.parentId != null && pickable.some(r => r.id === cur.parentId)) return [{ ...s, cursor: cur.parentId }]
      return [s]
    case 'right':
      return [cur ? { ...s, collapsed: s.collapsed.filter(c => c !== cur.id) } : s]
    case 'return':
      return cur && selectable(cur) ? [s, { kind: cur.claim === 'mine' ? 'unclaim' : 'claim', id: cur.id }] : [s]
    case 'backspace':
      return [{ ...s, query: '' }]
    case '/':
      return [{ ...s, mode: 'search' }]
    case 'a':
      return [{ ...s, mode: k.shift ? 'child' : 'sibling', draft: '' }]
    case 'A':
      return [{ ...s, mode: 'child', draft: '' }]
  }
  return [s]
}

export function initial(rows: PickRow[]): State {
  return { cursor: rows.find(selectable)?.id ?? null, top: 0, collapsed: [], mode: 'list', query: '', draft: '' }
}

const Picker: ClientModule<Props, State> = (props, surface) => {
  const { Box, Text } = surface.elements
  const rows = props.rows
  if (surface.state === undefined) {
    surface.onKey(k => {
      const [next, action] = press(surface.state ?? initial(rows), rows, k)
      surface.setState(next)
      if (action) surface.post(action)
    })
    surface.setState(initial(rows))
  }
  const s = surface.state ?? initial(rows)
  const shown = visible(rows, s.collapsed, s.query)
  const room = Math.max(1, surface.rows - CHROME)
  const at = Math.max(0, shown.findIndex(r => r.id === s.cursor))
  const start = at < s.top ? at : at >= s.top + room ? at - room + 1 : s.top
  if (start !== s.top) surface.setState({ ...s, top: start })

  const line = (r: ShownRow) => {
    const isCursor = r.id === s.cursor
    const fold = r.hasChildren ? (r.isCollapsed ? '▸ ' : '▾ ') : '  '
    const mark = r.claim === 'mine' ? '● ' : ''
    const isContext = r.claim === 'other' || (s.query !== '' && !r.isMatch)
    const suffix = r.claim === 'other' ? '  ⇢ other session' : ''
    return (
      <Text key={`row-${r.id}`} wrap="truncate-end" inverse={isCursor} dimColor={isContext} color={r.claim === 'mine' ? 'magenta' : undefined}>
        {`${'  '.repeat(r.depth)}${fold}${mark}${r.title}${suffix}`}
      </Text>
    )
  }

  const footer =
    s.mode === 'search'
      ? `/${s.query}▏  ⏎ done · ⌫ on empty exits`
      : s.mode === 'sibling' || s.mode === 'child'
        ? `new ${s.mode === 'child' ? 'child' : 'todo below'}: ${s.draft}▏  ⏎ add + claim · ⌫ on empty cancels`
        : `${s.query ? `/${s.query} · ⌫ clears · ` : ''}↑↓ move · ←→ fold · / search · ⏎ claim (● detach) · a/A add · esc close`

  return (
    <Box flexDirection="column">
      {props.error !== '' && <Text color="red">{props.error}</Text>}
      {props.error === '' && shown.length === 0 && (
        <Text dimColor>{s.query ? 'No matches.' : 'No open todos. Press a to add one.'}</Text>
      )}
      {shown.slice(start, start + room).map(line)}
      <Text dimColor wrap="truncate-end">
        {footer}
      </Text>
    </Box>
  )
}

export default Picker
