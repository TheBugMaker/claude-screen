/** One call of a run, as the run's line counts it. */
export type Call = { id: string; tool: string; status: 'running' | 'done' | 'errored' }

declare module 'claude-code' {
  interface PluginState {
    'ccs-traces': {
      /** The run a call is in, named by the run's first call; keyed by call id. */
      runOf: StateFamily<string>
      /** A run's calls, in order; keyed by run. */
      calls: StateFamily<Call[]>
      /** Whether a run lists its calls; keyed by run. */
      isOpen: StateFamily<boolean>
      /** Whether a call shows its whole input and output; keyed by call id. */
      isShown: StateFamily<boolean>
    }
  }
}
