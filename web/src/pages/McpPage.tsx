import { useEffect, useState } from 'react'
import { api } from '../api'
import type { McpServerConfig, McpTool } from '../types'

const emptyServer = (): McpServerConfig => ({
  id: '',
  name: '',
  transport: 'stdio',
  command: '',
  args: [],
  env: {},
  url: '',
  headers: {},
  enabled: true,
})

const kvToLines = (kv: Record<string, string>) =>
  Object.entries(kv)
    .map(([k, v]) => `${k}=${v}`)
    .join('\n')

const linesToKv = (text: string): Record<string, string> => {
  const out: Record<string, string> = {}
  for (const line of text.split('\n')) {
    const idx = line.indexOf('=')
    if (idx > 0) out[line.slice(0, idx).trim()] = line.slice(idx + 1).trim()
  }
  return out
}

export default function McpPage() {
  const [servers, setServers] = useState<McpServerConfig[]>([])
  const [editing, setEditing] = useState<McpServerConfig | null>(null)
  const [argsText, setArgsText] = useState('')
  const [envText, setEnvText] = useState('')
  const [headersText, setHeadersText] = useState('')
  const [error, setError] = useState('')
  const [tools, setTools] = useState<McpTool[] | null>(null)
  const [testing, setTesting] = useState(false)

  const reload = () => api.mcpServers().then(setServers).catch((e) => setError(e.message))
  useEffect(() => {
    reload()
  }, [])

  const startEdit = (s: McpServerConfig) => {
    setEditing({ ...s })
    setArgsText(s.args.join('\n'))
    setEnvText(kvToLines(s.env))
    setHeadersText(kvToLines(s.headers))
    setTools(null)
    setError('')
  }

  const save = async () => {
    if (!editing) return
    setError('')
    const payload: McpServerConfig = {
      ...editing,
      args: argsText.split('\n').map((a) => a.trim()).filter(Boolean),
      env: linesToKv(envText),
      headers: linesToKv(headersText),
    }
    try {
      if (editing.id) await api.updateMcp(payload)
      else await api.createMcp(payload)
      setEditing(null)
      reload()
    } catch (e: any) {
      setError(e.message)
    }
  }

  const remove = async (id: string) => {
    try {
      await api.deleteMcp(id)
      if (editing?.id === id) setEditing(null)
      reload()
    } catch (e: any) {
      setError(e.message)
    }
  }

  const testConnection = async (id: string) => {
    setTesting(true)
    setTools(null)
    setError('')
    try {
      const res = await api.mcpTools(id)
      setTools(res.tools)
    } catch (e: any) {
      setError(`Connection test failed: ${e.message}`)
    } finally {
      setTesting(false)
    }
  }

  return (
    <div className="page">
      {error && <div className="error-banner">{error}</div>}
      <div className="split">
        <div className="list-col">
          <div className="row between" style={{ marginBottom: 12 }}>
            <h2 style={{ margin: 0 }}>MCP Servers</h2>
            <button onClick={() => startEdit(emptyServer())}>+ Add server</button>
          </div>
          {servers.map((s) => (
            <div
              key={s.id}
              className={`card clickable ${editing?.id === s.id ? 'selected' : ''}`}
              onClick={() => startEdit(s)}
            >
              <div className="row between">
                <h3>{s.name}</h3>
                <span className={`badge ${s.enabled ? 'succeeded' : 'pending'}`}>
                  {s.enabled ? 'enabled' : 'disabled'}
                </span>
              </div>
              <div className="dim">
                {s.transport === 'stdio'
                  ? `stdio: ${s.command} ${s.args.join(' ')}`
                  : `http: ${s.url}`}
              </div>
            </div>
          ))}
          {servers.length === 0 && (
            <div className="dim card">
              No MCP servers yet. Add e.g. a Crunchyroll, web-search, e-mail or
              calendar MCP server here, then allow agents to use it via their
              agent card.
            </div>
          )}
        </div>
        <div className="detail-col">
          {editing ? (
            <div className="card">
              <h3>{editing.id ? `Edit: ${editing.name}` : 'New MCP server'}</h3>
              <label className="field">
                <span>Name</span>
                <input
                  value={editing.name}
                  onChange={(e) => setEditing({ ...editing, name: e.target.value })}
                  placeholder="e.g. crunchyroll"
                />
              </label>
              <label className="field">
                <span>Transport</span>
                <select
                  value={editing.transport}
                  onChange={(e) =>
                    setEditing({ ...editing, transport: e.target.value as 'stdio' | 'http' })
                  }
                >
                  <option value="stdio">stdio (local command)</option>
                  <option value="http">http (streamable HTTP endpoint)</option>
                </select>
              </label>
              {editing.transport === 'stdio' ? (
                <>
                  <label className="field">
                    <span>Command</span>
                    <input
                      value={editing.command}
                      onChange={(e) => setEditing({ ...editing, command: e.target.value })}
                      placeholder="e.g. npx / uvx / python"
                    />
                  </label>
                  <label className="field">
                    <span>Arguments (one per line)</span>
                    <textarea
                      rows={4}
                      value={argsText}
                      onChange={(e) => setArgsText(e.target.value)}
                      placeholder={'-y\n@some/mcp-server'}
                    />
                  </label>
                  <label className="field">
                    <span>Environment variables (KEY=VALUE, one per line)</span>
                    <textarea
                      rows={3}
                      value={envText}
                      onChange={(e) => setEnvText(e.target.value)}
                      placeholder="API_KEY=..."
                    />
                  </label>
                </>
              ) : (
                <>
                  <label className="field">
                    <span>URL</span>
                    <input
                      value={editing.url}
                      onChange={(e) => setEditing({ ...editing, url: e.target.value })}
                      placeholder="https://example.com/mcp"
                    />
                  </label>
                  <label className="field">
                    <span>Extra headers (KEY=VALUE, one per line)</span>
                    <textarea
                      rows={3}
                      value={headersText}
                      onChange={(e) => setHeadersText(e.target.value)}
                      placeholder="Authorization=Bearer ..."
                    />
                  </label>
                </>
              )}
              <div className="checkbox-row">
                <input
                  type="checkbox"
                  id="mcp-enabled"
                  checked={editing.enabled}
                  onChange={(e) => setEditing({ ...editing, enabled: e.target.checked })}
                />
                <label htmlFor="mcp-enabled">Enabled</label>
              </div>
              <div className="row">
                <button onClick={save}>Save</button>
                <button className="secondary" onClick={() => setEditing(null)}>
                  Cancel
                </button>
                {editing.id && (
                  <>
                    <button
                      className="secondary"
                      disabled={testing}
                      onClick={() => testConnection(editing.id)}
                    >
                      {testing ? 'Connecting…' : 'Test connection'}
                    </button>
                    <button className="danger" onClick={() => remove(editing.id)}>
                      Delete
                    </button>
                  </>
                )}
              </div>
              {tools && (
                <div style={{ marginTop: 14 }}>
                  <h3>✓ Connected — {tools.length} tools</h3>
                  {tools.map((t) => (
                    <div key={t.name} className="card">
                      <strong>{t.name}</strong>
                      <div className="dim">{t.description}</div>
                    </div>
                  ))}
                </div>
              )}
            </div>
          ) : (
            <div className="dim" style={{ padding: 30 }}>
              Select a server to edit it, or add a new one. Note: changes to a
              server that is already in use take effect on the next run.
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
