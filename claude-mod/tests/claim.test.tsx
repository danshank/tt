import { expect, test } from 'claude-code/testing'

import type { PickRow } from '../types'
import { initial, press } from '../hooks/picker'
import { claimNote, flatten, visible } from '../hooks/tree'

const TREE = {
  todos: [
    { id: 1, parent_id: null, position: 0, title: 'Todo Tree', done_at: null },
    { id: 2, parent_id: 1, position: 0, title: 'Improve selection', done_at: null },
    { id: 3, parent_id: 1, position: 1, title: 'Start from todo', done_at: null },
    { id: 4, parent_id: null, position: 1, title: 'Metrics', done_at: null },
    { id: 5, parent_id: 4, position: 0, title: 'OTel', done_at: null },
    { id: 6, parent_id: null, position: 2, title: 'Busy elsewhere', done_at: null },
    { id: 7, parent_id: null, position: 3, title: 'Done', done_at: '2026-10-01' },
    { id: 8, parent_id: 7, position: 0, title: 'under done', done_at: null },
  ],
  sessions: [
    { todo_id: 2, session_id: 'me' },
    { todo_id: 4, session_id: 'other' },
    { todo_id: 6, session_id: 'other' },
  ],
  tickets: [{ todo_id: 2, key: 'TT-1' }],
}

const rows: PickRow[] = flatten(TREE, 'me')
const ids = (rs: { id: number }[]) => rs.map(r => r.id)
const keys = (start: ReturnType<typeof initial>, ...ks: string[]) =>
  ks.reduce<[ReturnType<typeof initial>, unknown]>(([s], k) => press(s, rows, { key: k }) as [ReturnType<typeof initial>, unknown], [start, undefined])

test('flatten drops done subtrees and leaves claimed elsewhere, keeps claimed parents as context', async () => {
  expect(ids(rows)).toEqual([1, 2, 3, 4, 5])
  expect(rows.find(r => r.id === 2)?.claim).toBe('mine')
  expect(rows.find(r => r.id === 4)?.claim).toBe('other')
  expect(rows.find(r => r.id === 5)?.path).toBe('Metrics › OTel')
})

test('search matches every term against the path and keeps ancestors as context', async () => {
  const hit = visible(rows, [], 'tree sel')
  expect(ids(hit)).toEqual([1, 2])
  expect(hit.map(r => r.isMatch)).toEqual([false, true])
  expect(ids(visible(rows, [], 'metrics'))).toEqual([4, 5])
  expect(visible(rows, [], 'metrics').find(r => r.id === 4)?.isMatch).toBe(false)
})

test('collapse hides a subtree unless a query is active', async () => {
  expect(ids(visible(rows, [1], ''))).toEqual([1, 4, 5])
  expect(ids(visible(rows, [1], 'start'))).toEqual([1, 3])
})

test('cursor starts on first row and skips todos claimed elsewhere', async () => {
  const s = initial(rows)
  expect(s.cursor).toBe(1)
  const [down] = keys(s, 'down', 'down', 'down')
  expect(down.cursor).toBe(5)
})

test('return claims a free todo and detaches a mine one', async () => {
  expect(keys(initial(rows), 'return')[1]).toEqual({ kind: 'claim', id: 1 })
  expect(keys(initial(rows), 'down', 'return')[1]).toEqual({ kind: 'unclaim', id: 2 })
})

test('slash searches, return keeps the filter, backspace clears it', async () => {
  const [searched] = keys(initial(rows), '/', 's', 't', 'a', 'r', 't', 'return')
  expect(searched.mode).toBe('list')
  expect(searched.cursor).toBe(3)
  expect(ids(visible(rows, searched.collapsed, searched.query))).toEqual([1, 3])
  expect(keys(searched, 'backspace')[0].query).toBe('')
})

test('a adds below, A adds inside the selected todo', async () => {
  expect(keys(initial(rows), 'a', 'h', 'i', 'return')[1]).toEqual({ kind: 'add', title: 'hi', after: 1 })
  expect(keys(initial(rows), 'A', 'h', 'i', 'return')[1]).toEqual({ kind: 'add', title: 'hi', parent: 1 })
  const [cancelled, none] = keys(initial(rows), 'a', 'backspace')
  expect(cancelled.mode).toBe('list')
  expect(none).toBeUndefined()
})

test('left folds, then climbs to the parent', async () => {
  const [folded] = keys(initial(rows), 'left')
  expect(folded.collapsed).toEqual([1])
  const [climbed] = keys(initial(rows), 'down', 'left')
  expect(climbed.cursor).toBe(1)
})

test('claim note names the path and tickets', async () => {
  expect(claimNote(rows.find(r => r.id === 2)!, 'Claimed')).toBe('Claimed tt todo #2: Todo Tree › Improve selection (Linear: TT-1)')
})

test('pane draws the picker client', async $ => {
  for (const surface of ['terminal', 'desktop'] as const) {
    const ui = await $.ui.mount({
      plugin: 'tt-claim',
      surface,
      component: 'Pane',
      props: { title: 'Claim a todo', isFocused: true, bodyColumns: 80, placement: 'dock' } as never,
      requestId: 'tt-claim',
    })
    expect(await ui.find({ key: 'picker' })).toBeDefined()
    await ui.unmount()
  }
})
