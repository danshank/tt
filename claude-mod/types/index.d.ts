export type PickRow = { id: number; depth: number; title: string; isClaimed: boolean }

declare module 'claude-code' {
  interface PluginState {
    'tt-claim': { rows: PickRow[]; error: string }
  }
}
