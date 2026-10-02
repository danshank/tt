import type { PickRow } from '../types'

type TodoJson = { id: number; parent_id: number | null; position: number; title: string; done_at: string | null }
type LinkJson = { todo_id: number; session_id: string }
export type TreeJson = { todos: TodoJson[]; sessions: LinkJson[] }

/** Open todos depth-first; a done todo hides its subtree, as in the tt TUI. */
export function flatten(tree: TreeJson, sessionId: string): PickRow[] {
  const claimed = new Set(tree.sessions.filter(s => s.session_id === sessionId).map(s => s.todo_id))
  const out: PickRow[] = []
  const walk = (parent: number | null, depth: number) => {
    tree.todos
      .filter(t => t.parent_id === parent && t.done_at === null)
      .sort((a, b) => a.position - b.position || a.id - b.id)
      .forEach(t => {
        out.push({ id: t.id, depth, title: t.title, isClaimed: claimed.has(t.id) })
        walk(t.id, depth + 1)
      })
  }
  walk(null, 0)
  return out
}
