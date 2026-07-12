import { useEffect, useState } from 'react'
import { api } from './api'
import type { ServerConfig } from './types'
import AgentsPage from './pages/AgentsPage'
import McpPage from './pages/McpPage'
import RunsPage from './pages/RunsPage'
import WorkflowsPage from './pages/WorkflowsPage'

type Tab = 'workflows' | 'agents' | 'mcp' | 'runs'

export default function App() {
  const [tab, setTab] = useState<Tab>('workflows')
  const [config, setConfig] = useState<ServerConfig | null>(null)
  const [viewRunId, setViewRunId] = useState<string | null>(null)

  useEffect(() => {
    api.config().then(setConfig).catch(() => setConfig(null))
  }, [])

  const openRun = (runId: string) => {
    setViewRunId(runId)
    setTab('runs')
  }

  return (
    <>
      <header className="topbar">
        <span className="logo">SAIOAWE</span>
        <nav>
          {(['workflows', 'agents', 'mcp', 'runs'] as Tab[]).map((t) => (
            <button
              key={t}
              className={tab === t ? 'active' : ''}
              onClick={() => setTab(t)}
            >
              {t === 'mcp' ? 'MCP Servers' : t[0].toUpperCase() + t.slice(1)}
            </button>
          ))}
        </nav>
        <span className="spacer" />
        <span className="conn">
          {config
            ? `Ollama: ${config.ollama_url} · default model: ${config.default_model}`
            : 'Ollama: not reachable'}
        </span>
      </header>
      {tab === 'workflows' && <WorkflowsPage config={config} onOpenRun={openRun} />}
      {tab === 'agents' && <AgentsPage config={config} />}
      {tab === 'mcp' && <McpPage />}
      {tab === 'runs' && <RunsPage viewRunId={viewRunId} onViewRun={setViewRunId} />}
    </>
  )
}
