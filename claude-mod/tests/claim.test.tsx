import { expect, test } from 'claude-code/testing'

import { flatten } from '../hooks/tree'

test('flatten nests open todos and marks this session', async () => {
  const rows = flatten(
    {
      todos: [
        { id: 1, parent_id: null, position: 0, title: 'a', done_at: null },
        { id: 2, parent_id: 1, position: 1, title: 'b', done_at: null },
        { id: 3, parent_id: 1, position: 0, title: 'c', done_at: '2026-10-01' },
        { id: 4, parent_id: 3, position: 0, title: 'hidden', done_at: null },
      ],
      sessions: [{ todo_id: 2, session_id: 's1' }, { todo_id: 1, session_id: 'other' }],
    },
    's1',
  )
  expect(rows).toEqual([
    { id: 1, depth: 0, title: 'a', isClaimed: false },
    { id: 2, depth: 1, title: 'b', isClaimed: true },
  ])
})

test('empty pane offers the new-todo input', async $ => {
  const ui = await $.ui.mount({ plugin: 'tt-claim', surface: 'terminal', component: 'Pane', props: {}, requestId: 'tt-claim' })
  expect(await ui.find({ type: 'Text', text: /No open todos/ })).toBeDefined()
  expect(await ui.find({ key: 'new' })).toBeDefined()
  await ui.unmount()
})
