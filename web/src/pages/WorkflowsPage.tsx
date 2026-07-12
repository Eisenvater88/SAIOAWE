import { useEffect, useState } from 'react'
import { api } from '../api'
import type { ServerConfig, Workflow } from '../types'
import WorkflowEditor from '../editor/WorkflowEditor'

export default function WorkflowsPage({
  config,
  onOpenRun,
}: {
  config: ServerConfig | null
  onOpenRun: (runId: string) => void
}) {
  const [workflows, setWorkflows] = useState<Workflow[]>([])
  const [editing, setEditing] = useState<Workflow | null>(null)
  const [newName, setNewName] = useState('')
  const [error, setError] = useState('')

  const reload = () => api.workflows().then(setWorkflows).catch((e) => setError(e.message))
  useEffect(() => {
    reload()
  }, [])

  const create = async () => {
    if (!newName.trim()) return
    setError('')
    try {
      const wf = await api.createWorkflow({
        name: newName.trim(),
        description: '',
        graph: { nodes: [], edges: [] },
        max_steps: 25,
      })
      setNewName('')
      reload()
      setEditing(wf)
    } catch (e: any) {
      setError(e.message)
    }
  }

  const remove = async (id: string) => {
    setError('')
    try {
      await api.deleteWorkflow(id)
      reload()
    } catch (e: any) {
      setError(e.message)
    }
  }

  if (editing) {
    return (
      <div className="page no-pad">
        <WorkflowEditor
          workflow={editing}
          onBack={() => {
            setEditing(null)
            reload()
          }}
          onOpenRun={onOpenRun}
        />
      </div>
    )
  }

  return (
    <div className="page">
      {error && <div className="error-banner">{error}</div>}
      <div className="row" style={{ marginBottom: 16, maxWidth: 560 }}>
        <input
          placeholder="New workflow name…"
          value={newName}
          onChange={(e) => setNewName(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && create()}
        />
        <button onClick={create}>+ Create</button>
      </div>
      {workflows.map((w) => (
        <div key={w.id} className="card" style={{ maxWidth: 560 }}>
          <div className="row between">
            <div>
              <h3>{w.name}</h3>
              <div className="dim">
                {w.graph.nodes.length} agents · {w.graph.edges.length} connections
              </div>
            </div>
            <div className="row">
              <button onClick={() => setEditing(w)}>Open</button>
              <button className="danger small" onClick={() => remove(w.id)}>
                Delete
              </button>
            </div>
          </div>
        </div>
      ))}
      {workflows.length === 0 && (
        <div className="dim card" style={{ maxWidth: 560 }}>
          No workflows yet. Create one, then drag agents onto the canvas and
          connect them — each arrow feeds one agent's output into the next.
        </div>
      )}
    </div>
  )
}
