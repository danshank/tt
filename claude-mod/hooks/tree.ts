import type { PickRow } from '../types'

type TodoJson = { id: number; parent_id: number | null; position: number; title: string; done_at: string | null }
type LinkJson = { todo_id: number; session_id: string }
type TicketJson = { todo_id: number; key: string }
export type TreeJson = { todos: TodoJson[]; sessions: LinkJson[]; tickets?: TicketJson[] }

/**
 * Open todos depth-first; a done todo hides its subtree, as in the tt TUI.
 * Todos claimed by another session stay only as context for open descendants.
 */
export function flatten(tree: TreeJson, sessionId: string): PickRow[] {
  const owner = new Map(tree.sessions.map(s => [s.todo_id, s.session_id]))
  const tickets = tree.tickets ?? []
  const walk = (parent: number | null, depth: number, path: string[], parentId: number | null): PickRow[] =>
    tree.todos
      .filter(t => t.parent_id === parent && t.done_at === null)
      .sort((a, b) => a.position - b.position || a.id - b.id)
      .flatMap(t => {
        const p = [...path, t.title]
        const kids = walk(t.id, depth + 1, p, t.id)
        const o = owner.get(t.id)
        const claim = o === undefined ? 'free' : o === sessionId ? 'mine' : 'other'
        if (claim === 'other' && kids.length === 0) return []
        const row: PickRow = {
          id: t.id,
          parentId,
          depth,
          title: t.title,
          path: p.join(' › '),
          claim,
          hasChildren: kids.length > 0,
          tickets: tickets.filter(k => k.todo_id === t.id).map(k => k.key),
        }
        return [row, ...kids]
      })
  return walk(null, 0, [], null)
}

export type ShownRow = PickRow & { isMatch: boolean; isCollapsed: boolean }

/**
 * Rows to draw. No query: the tree minus collapsed subtrees. With a query: todos whose path holds
 * every term (never one claimed elsewhere), plus their ancestors as context; collapse is ignored.
 */
export function visible(rows: PickRow[], collapsed: number[], query: string): ShownRow[] {
  const terms = query.toLowerCase().split(/\s+/).filter(Boolean)
  const byId = new Map(rows.map(r => [r.id, r]))
  if (terms.length === 0) {
    const hidden = new Set<number>()
    return rows.flatMap(r => {
      if (r.parentId !== null && (hidden.has(r.parentId) || collapsed.includes(r.parentId))) {
        hidden.add(r.id)
        return []
      }
      return [{ ...r, isMatch: false, isCollapsed: r.hasChildren && collapsed.includes(r.id) }]
    })
  }
  const matches = new Set(
    rows.filter(r => r.claim !== 'other' && terms.every(t => r.path.toLowerCase().includes(t))).map(r => r.id),
  )
  const keep = new Set<number>()
  for (const id of matches) {
    for (let cur = byId.get(id); cur && !keep.has(cur.id); cur = cur.parentId === null ? undefined : byId.get(cur.parentId)) {
      keep.add(cur.id)
    }
  }
  return rows.filter(r => keep.has(r.id)).map(r => ({ ...r, isMatch: matches.has(r.id), isCollapsed: false }))
}

export const selectable = (r: PickRow) => r.claim !== 'other'

/** One line for the model: what this session now owns, or gave up. */
export function claimNote(row: PickRow, verb: 'Claimed' | 'Detached from'): string {
  const linear = row.tickets.length > 0 ? ` (Linear: ${row.tickets.join(', ')})` : ''
  return `${verb} tt todo #${row.id}: ${row.path}${linear}`
}
