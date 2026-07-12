import type {
  AgentCard,
  McpServerConfig,
  McpTool,
  NodeRun,
  Run,
  RunEvent,
  Schedule,
  ServerConfig,
  Workflow,
} from './types'

async function req<T>(path: string, options?: RequestInit): Promise<T> {
  const res = await fetch(path, {
    headers: { 'Content-Type': 'application/json' },
    ...options,
  })
  if (!res.ok) {
    let msg = `${res.status} ${res.statusText}`
    try {
      const body = await res.json()
      if (body?.error) msg = body.error
    } catch {
      /* keep default message */
    }
    throw new Error(msg)
  }
  return res.json()
}

export const api = {
  config: () => req<ServerConfig>('/api/config'),

  agents: () => req<AgentCard[]>('/api/agents'),
  createAgent: (a: Partial<AgentCard>) =>
    req<AgentCard>('/api/agents', { method: 'POST', body: JSON.stringify(a) }),
  updateAgent: (a: AgentCard) =>
    req<AgentCard>(`/api/agents/${a.id}`, { method: 'PUT', body: JSON.stringify(a) }),
  deleteAgent: (id: string) => req(`/api/agents/${id}`, { method: 'DELETE' }),

  workflows: () => req<Workflow[]>('/api/workflows'),
  workflow: (id: string) => req<Workflow>(`/api/workflows/${id}`),
  createWorkflow: (w: Partial<Workflow>) =>
    req<Workflow>('/api/workflows', { method: 'POST', body: JSON.stringify(w) }),
  updateWorkflow: (w: Workflow) =>
    req<Workflow>(`/api/workflows/${w.id}`, { method: 'PUT', body: JSON.stringify(w) }),
  deleteWorkflow: (id: string) => req(`/api/workflows/${id}`, { method: 'DELETE' }),
  runWorkflow: (id: string, input: string) =>
    req<Run>(`/api/workflows/${id}/run`, { method: 'POST', body: JSON.stringify({ input }) }),

  mcpServers: () => req<McpServerConfig[]>('/api/mcp-servers'),
  createMcp: (m: Partial<McpServerConfig>) =>
    req<McpServerConfig>('/api/mcp-servers', { method: 'POST', body: JSON.stringify(m) }),
  updateMcp: (m: McpServerConfig) =>
    req<McpServerConfig>(`/api/mcp-servers/${m.id}`, { method: 'PUT', body: JSON.stringify(m) }),
  deleteMcp: (id: string) => req(`/api/mcp-servers/${id}`, { method: 'DELETE' }),
  mcpTools: (id: string) => req<{ tools: McpTool[] }>(`/api/mcp-servers/${id}/tools`),

  schedules: () => req<Schedule[]>('/api/schedules'),
  createSchedule: (s: Partial<Schedule>) =>
    req<Schedule>('/api/schedules', { method: 'POST', body: JSON.stringify(s) }),
  updateSchedule: (s: Schedule) =>
    req<Schedule>(`/api/schedules/${s.id}`, { method: 'PUT', body: JSON.stringify(s) }),
  deleteSchedule: (id: string) => req(`/api/schedules/${id}`, { method: 'DELETE' }),

  runs: (workflowId?: string) =>
    req<Run[]>(`/api/runs${workflowId ? `?workflow_id=${workflowId}` : ''}`),
  run: (id: string) => req<{ run: Run; node_runs: NodeRun[] }>(`/api/runs/${id}`),
  cancelRun: (id: string) => req(`/api/runs/${id}/cancel`, { method: 'POST' }),
}

// ---- live run events (single shared EventSource) ----

type Listener = (ev: RunEvent) => void
const listeners = new Set<Listener>()
let source: EventSource | null = null

function ensureSource() {
  if (source) return
  source = new EventSource('/api/events')
  source.addEventListener('run', (e) => {
    try {
      const ev = JSON.parse((e as MessageEvent).data) as RunEvent
      listeners.forEach((l) => l(ev))
    } catch {
      /* ignore malformed events */
    }
  })
  source.onerror = () => {
    // Browser auto-reconnects EventSource; nothing to do.
  }
}

export function subscribeRunEvents(l: Listener): () => void {
  ensureSource()
  listeners.add(l)
  return () => listeners.delete(l)
}
