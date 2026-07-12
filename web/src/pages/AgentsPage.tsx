import { useEffect, useState } from 'react'
import { api } from '../api'
import type { AgentCard, McpServerConfig, ServerConfig } from '../types'

const emptyCard = (): AgentCard => ({
  id: '',
  name: '',
  description: '',
  model: '',
  system_prompt: '',
  mcp_servers: [],
  temperature: null,
  max_tool_iterations: 10,
})

export default function AgentsPage({ config }: { config: ServerConfig | null }) {
  const [agents, setAgents] = useState<AgentCard[]>([])
  const [servers, setServers] = useState<McpServerConfig[]>([])
  const [editing, setEditing] = useState<AgentCard | null>(null)
  const [error, setError] = useState('')

  const reload = () => {
    api.agents().then(setAgents).catch((e) => setError(e.message))
    api.mcpServers().then(setServers).catch(() => {})
  }
  useEffect(reload, [])

  const save = async () => {
    if (!editing) return
    setError('')
    try {
      const payload = {
        ...editing,
        model: editing.model?.trim() ? editing.model : null,
      }
      if (editing.id) await api.updateAgent(payload)
      else await api.createAgent(payload)
      setEditing(null)
      reload()
    } catch (e: any) {
      setError(e.message)
    }
  }

  const remove = async (id: string) => {
    setError('')
    try {
      await api.deleteAgent(id)
      if (editing?.id === id) setEditing(null)
      reload()
    } catch (e: any) {
      setError(e.message)
    }
  }

  const toggleServer = (id: string) => {
    if (!editing) return
    const has = editing.mcp_servers.includes(id)
    setEditing({
      ...editing,
      mcp_servers: has
        ? editing.mcp_servers.filter((s) => s !== id)
        : [...editing.mcp_servers, id],
    })
  }

  return (
    <div className="page">
      {error && <div className="error-banner">{error}</div>}
      <div className="split">
        <div className="list-col">
          <div className="row between" style={{ marginBottom: 12 }}>
            <h2 style={{ margin: 0 }}>Agent Cards</h2>
            <button onClick={() => setEditing(emptyCard())}>+ New agent</button>
          </div>
          {agents.map((a) => (
            <div
              key={a.id}
              className={`card clickable ${editing?.id === a.id ? 'selected' : ''}`}
              onClick={() => setEditing({ ...a })}
            >
              <h3>{a.name}</h3>
              <div className="dim">{a.description || 'no description'}</div>
              <div className="dim">
                model: {a.model || `(default: ${config?.default_model ?? '?'})`} · MCP:{' '}
                {a.mcp_servers.length}
              </div>
            </div>
          ))}
          {agents.length === 0 && (
            <div className="dim card">
              No agent cards yet. An agent card describes one agent: its role
              (system prompt), model and the MCP servers it may use.
            </div>
          )}
        </div>
        <div className="detail-col">
          {editing ? (
            <div className="card">
              <h3>{editing.id ? `Edit: ${editing.name}` : 'New agent card'}</h3>
              <label className="field">
                <span>Name</span>
                <input
                  value={editing.name}
                  onChange={(e) => setEditing({ ...editing, name: e.target.value })}
                  placeholder="e.g. Anime History Fetcher"
                />
              </label>
              <label className="field">
                <span>Description</span>
                <input
                  value={editing.description}
                  onChange={(e) => setEditing({ ...editing, description: e.target.value })}
                  placeholder="What this agent is for"
                />
              </label>
              <label className="field">
                <span>Model (empty = server default)</span>
                {config?.models?.length ? (
                  <select
                    value={editing.model ?? ''}
                    onChange={(e) => setEditing({ ...editing, model: e.target.value })}
                  >
                    <option value="">(default: {config.default_model})</option>
                    {config.models.map((m) => (
                      <option key={m} value={m}>
                        {m}
                      </option>
                    ))}
                  </select>
                ) : (
                  <input
                    value={editing.model ?? ''}
                    onChange={(e) => setEditing({ ...editing, model: e.target.value })}
                    placeholder="e.g. qwen2.5:14b"
                  />
                )}
              </label>
              <label className="field">
                <span>System prompt (the agent's role & behavior)</span>
                <textarea
                  rows={7}
                  value={editing.system_prompt}
                  onChange={(e) => setEditing({ ...editing, system_prompt: e.target.value })}
                  placeholder="You are an expert at ..."
                />
              </label>
              <div className="field">
                <span
                  style={{
                    color: 'var(--text-dim)',
                    fontSize: 12,
                    textTransform: 'uppercase',
                    letterSpacing: '0.04em',
                  }}
                >
                  MCP servers this agent may use
                </span>
                {servers.length === 0 && (
                  <div className="dim">None configured yet - see the MCP Servers tab.</div>
                )}
                {servers.map((s) => (
                  <div key={s.id} className="checkbox-row">
                    <input
                      type="checkbox"
                      id={`mcp-${s.id}`}
                      checked={editing.mcp_servers.includes(s.id)}
                      onChange={() => toggleServer(s.id)}
                    />
                    <label htmlFor={`mcp-${s.id}`}>
                      {s.name} {!s.enabled && <span className="dim">(disabled)</span>}
                    </label>
                  </div>
                ))}
              </div>
              <div className="row">
                <label className="field" style={{ flex: 1 }}>
                  <span>Temperature (empty = default)</span>
                  <input
                    type="number"
                    step="0.1"
                    min="0"
                    max="2"
                    value={editing.temperature ?? ''}
                    onChange={(e) =>
                      setEditing({
                        ...editing,
                        temperature: e.target.value === '' ? null : Number(e.target.value),
                      })
                    }
                  />
                </label>
                <label className="field" style={{ flex: 1 }}>
                  <span>Max tool iterations</span>
                  <input
                    type="number"
                    min="1"
                    max="50"
                    value={editing.max_tool_iterations}
                    onChange={(e) =>
                      setEditing({ ...editing, max_tool_iterations: Number(e.target.value) || 10 })
                    }
                  />
                </label>
              </div>
              <div className="row">
                <button onClick={save}>Save</button>
                <button className="secondary" onClick={() => setEditing(null)}>
                  Cancel
                </button>
                {editing.id && (
                  <button className="danger" onClick={() => remove(editing.id)}>
                    Delete
                  </button>
                )}
              </div>
            </div>
          ) : (
            <div className="dim" style={{ padding: 30 }}>
              Select an agent card to edit it, or create a new one.
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
