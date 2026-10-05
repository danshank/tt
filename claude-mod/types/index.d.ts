export type PickRow = {
  id: number
  parentId: number | null
  depth: number
  title: string
  path: string
  claim: 'free' | 'mine' | 'other'
  hasChildren: boolean
  tickets: string[]
}

/** What the picker Client posts to the hooks module. */
export type PickAction =
  | { kind: 'claim' | 'unclaim'; id: number }
  | { kind: 'add'; title: string; after?: number; parent?: number }

declare module 'claude-code' {
  interface PluginState {
    'tt-claim': { rows: PickRow[]; error: string }
  }
}
