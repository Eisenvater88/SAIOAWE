import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  addEdge,
  Background,
  Controls,
  MarkerType,
  MiniMap,
  ReactFlow,
  useEdgesState,
  useNodesState,
  type Connection,
  type Edge,
  type Node,
} from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import { api, subscribeRunEvents } from '../api'
import type { AgentCard, ConditionKind, RunEvent, Schedule, Workflow } from '../types'
import AgentNode, { type AgentNodeData } from './AgentNode'
import FileNode from './FileNode'
import FileDestNode from './FileDestNode'

const nodeTypes = { agent: AgentNode, file: FileNode, file_dest: FileDestNode }

const fileLabel = (path: string, fallback = 'File source') => {
  const base = path.replace(/[\\/]+$/, '').split(/[\\/]/).pop()
  return base || fallback
}

let idCounter = 0
const freshId = () => `n${Date.now().toString(36)}_${idCounter++}`

type RFNode = Node<AgentNodeData>

type EdgeData = {
  conditionKind: ConditionKind
  condition: string
  negate: boolean
  live?: 'taken' | 'silent'
}

const edgeLabel = (d: EdgeData) => {
  if (!d.conditionKind || d.conditionKind === 'always') return undefined
  const text = d.condition.length > 26 ? d.condition.slice(0, 26) + '…' : d.condition
  return `${d.negate ? 'NOT ' : ''}${d.conditionKind}: ${text}`
}

function styleEdge(e: Edge): Edge {
  const d = (e.data ?? {}) as EdgeData
  const conditional = d.conditionKind && d.conditionKind !== 'always'
  const stroke =
    d.live === 'taken' ? 'var(--green)' : d.live === 'silent' ? '#444a5a' : undefined
  return {
    ...e,
    label: edgeLabel(d),
    animated: d.live === 'taken',
    markerEnd: { type: MarkerType.ArrowClosed, color: stroke },
    style: {
      ...(conditional ? { strokeDasharray: '6 3' } : {}),
      ...(stroke ? { stroke } : {}),
      ...(d.live === 'silent' ? { opacity: 0.4 } : {}),
    },
    labelStyle: { fill: 'var(--text-dim)', fontSize: 10 },
    labelBgStyle: { fill: 'var(--bg-panel)' },
  }
}

function toFlow(wf: Workflow, agents: AgentCard[]): { nodes: RFNode[]; edges: Edge[] } {
  const agentName = (id: string) => agents.find((a) => a.id === id)?.name ?? '⚠ missing agent'
  return {
    nodes: wf.graph.nodes.map((n) => {
      const kind = n.kind ?? 'agent'
      return {
        id: n.id,
        type: kind,
        position: n.position,
        data: {
          kind,
          label:
            kind === 'file'
              ? fileLabel(n.file_path ?? '')
              : kind === 'file_dest'
                ? fileLabel(n.file_path ?? '', 'File destination')
                : agentName(n.agent_card_id),
          agentCardId: n.agent_card_id ?? '',
          instructions: n.instructions ?? '',
          filePath: n.file_path ?? '',
          append: n.append ?? false,
        },
      }
    }),
    edges: wf.graph.edges.map((e) =>
      styleEdge({
        id: e.id,
        source: e.source,
        target: e.target,
        data: {
          conditionKind: e.condition_kind ?? 'always',
          condition: e.condition ?? '',
          negate: e.negate ?? false,
        },
      }),
    ),
  }
}

export default function WorkflowEditor({
  workflow,
  onBack,
  onOpenRun,
}: {
  workflow: Workflow
  onBack: () => void
  onOpenRun: (runId: string) => void
}) {
  const [agents, setAgents] = useState<AgentCard[]>([])
  const [nodes, setNodes, onNodesChange] = useNodesState<RFNode>([])
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>([])
  const [name, setName] = useState(workflow.name)
  const [maxSteps, setMaxSteps] = useState(workflow.max_steps || 25)
  const [selectedNode, setSelectedNode] = useState<string | null>(null)
  const [selectedEdge, setSelectedEdge] = useState<string | null>(null)
  const [error, setError] = useState('')
  const [saved, setSaved] = useState(false)
  const [runInput, setRunInput] = useState('')
  const [activeRunId, setActiveRunId] = useState<string | null>(null)
  const [runLog, setRunLog] = useState<RunEvent[]>([])
  const [schedules, setSchedules] = useState<Schedule[]>([])
  const [newCron, setNewCron] = useState('0 8 * * *')
  const [newCronInput, setNewCronInput] = useState('')
  const logRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    api.agents().then((a) => {
      setAgents(a)
      const flow = toFlow(workflow, a)
      setNodes(flow.nodes)
      setEdges(flow.edges)
    })
    reloadSchedules()
  }, [workflow.id])

  const reloadSchedules = () =>
    api
      .schedules()
      .then((all) => setSchedules(all.filter((s) => s.workflow_id === workflow.id)))
      .catch(() => {})

  // Live node/edge status while a run is active.
  useEffect(() => {
    if (!activeRunId) return
    return subscribeRunEvents((ev) => {
      if (ev.run_id !== activeRunId) return
      setRunLog((log) => [...log, ev].slice(-400))
      if (ev.kind === 'edge_resolved') {
        setEdges((es) =>
          es.map((e) =>
            e.id === ev.data.edge_id
              ? styleEdge({
                  ...e,
                  data: { ...(e.data as EdgeData), live: ev.data.taken ? 'taken' : 'silent' },
                })
              : e,
          ),
        )
      }
      if (ev.node_id) {
        const status =
          ev.kind === 'node_started'
            ? 'running'
            : ev.kind === 'node_skipped'
              ? 'skipped'
              : ev.kind === 'node_finished'
                ? ev.data?.status
                : undefined
        if (status) {
          const activation = ev.data?.activation
          setNodes((ns) =>
            ns.map((n) =>
              n.id === ev.node_id
                ? { ...n, data: { ...n.data, status, activation } }
                : n,
            ),
          )
        }
      }
      setTimeout(() => logRef.current?.scrollTo({ top: 999999 }), 30)
    })
  }, [activeRunId, setNodes, setEdges])

  const onConnect = useCallback(
    (c: Connection) =>
      setEdges((eds) =>
        addEdge(
          styleEdge({
            ...c,
            id: freshId(),
            data: { conditionKind: 'always', condition: '', negate: false },
          } as Edge),
          eds,
        ),
      ),
    [setEdges],
  )

  const addAgentNode = (agent: AgentCard) => {
    const id = freshId()
    setNodes((ns) => [
      ...ns,
      {
        id,
        type: 'agent',
        position: { x: 80 + (ns.length % 4) * 260, y: 80 + Math.floor(ns.length / 4) * 140 },
        data: {
          kind: 'agent',
          label: agent.name,
          agentCardId: agent.id,
          instructions: '',
          filePath: '',
        },
      },
    ])
    setSelectedNode(id)
    setSelectedEdge(null)
  }

  const addFileNode = () => {
    const id = freshId()
    setNodes((ns) => [
      ...ns,
      {
        id,
        type: 'file',
        position: { x: 80 + (ns.length % 4) * 260, y: 80 + Math.floor(ns.length / 4) * 140 },
        data: { kind: 'file', label: 'File source', agentCardId: '', instructions: '', filePath: '' },
      },
    ])
    setSelectedNode(id)
    setSelectedEdge(null)
  }

  const addFileDestNode = () => {
    const id = freshId()
    setNodes((ns) => [
      ...ns,
      {
        id,
        type: 'file_dest',
        position: { x: 80 + (ns.length % 4) * 260, y: 80 + Math.floor(ns.length / 4) * 140 },
        data: {
          kind: 'file_dest',
          label: 'File destination',
          agentCardId: '',
          instructions: '',
          filePath: '',
          append: false,
        },
      },
    ])
    setSelectedNode(id)
    setSelectedEdge(null)
  }

  const currentWorkflow = (): Workflow => ({
    ...workflow,
    name,
    max_steps: maxSteps,
    graph: {
      nodes: nodes.map((n) => ({
        id: n.id,
        kind: n.data.kind ?? 'agent',
        agent_card_id: n.data.agentCardId,
        instructions: n.data.instructions,
        file_path: n.data.filePath ?? '',
        append: n.data.append ?? false,
        position: { x: n.position.x, y: n.position.y },
      })),
      edges: edges.map((e) => {
        const d = (e.data ?? {}) as EdgeData
        return {
          id: e.id,
          source: e.source,
          target: e.target,
          condition_kind: d.conditionKind ?? 'always',
          condition: d.condition ?? '',
          negate: d.negate ?? false,
        }
      }),
    },
  })

  const save = async () => {
    setError('')
    try {
      await api.updateWorkflow(currentWorkflow())
      setSaved(true)
      setTimeout(() => setSaved(false), 1500)
    } catch (e: any) {
      setError(e.message)
    }
  }

  const run = async () => {
    setError('')
    try {
      await api.updateWorkflow(currentWorkflow()) // run what you see
      setNodes((ns) =>
        ns.map((n) => ({ ...n, data: { ...n.data, status: undefined, activation: undefined } })),
      )
      setEdges((es) =>
        es.map((e) => styleEdge({ ...e, data: { ...(e.data as EdgeData), live: undefined } })),
      )
      setRunLog([])
      const r = await api.runWorkflow(workflow.id, runInput)
      setActiveRunId(r.id)
    } catch (e: any) {
      setError(e.message)
    }
  }

  const addSchedule = async () => {
    setError('')
    try {
      await api.createSchedule({
        workflow_id: workflow.id,
        cron: newCron,
        input: newCronInput,
        enabled: true,
      })
      reloadSchedules()
    } catch (e: any) {
      setError(e.message)
    }
  }

  const toggleSchedule = async (s: Schedule) => {
    try {
      await api.updateSchedule({ ...s, enabled: !s.enabled })
      reloadSchedules()
    } catch (e: any) {
      setError(e.message)
    }
  }

  const deleteSchedule = async (id: string) => {
    try {
      await api.deleteSchedule(id)
      reloadSchedules()
    } catch (e: any) {
      setError(e.message)
    }
  }

  const selected = useMemo(() => nodes.find((n) => n.id === selectedNode), [nodes, selectedNode])
  const selEdge = useMemo(() => edges.find((e) => e.id === selectedEdge), [edges, selectedEdge])
  const selEdgeData = (selEdge?.data ?? {}) as EdgeData
  const nodeName = (id?: string) => nodes.find((n) => n.id === id)?.data.label ?? '?'

  const updateSelected = (patch: Partial<AgentNodeData>) => {
    if (!selectedNode) return
    setNodes((ns) =>
      ns.map((n) => {
        if (n.id !== selectedNode) return n
        const data = { ...n.data, ...patch }
        if (patch.agentCardId) {
          data.label = agents.find((a) => a.id === patch.agentCardId)?.name ?? '⚠ missing agent'
        }
        if (patch.filePath !== undefined) {
          data.label = fileLabel(
            patch.filePath,
            n.data.kind === 'file_dest' ? 'File destination' : 'File source',
          )
        }
        return { ...n, data }
      }),
    )
  }

  const updateSelectedEdge = (patch: Partial<EdgeData>) => {
    if (!selectedEdge) return
    setEdges((es) =>
      es.map((e) =>
        e.id === selectedEdge
          ? styleEdge({ ...e, data: { ...(e.data as EdgeData), ...patch } })
          : e,
      ),
    )
  }

  const deleteSelected = () => {
    if (selectedNode) {
      setNodes((ns) => ns.filter((n) => n.id !== selectedNode))
      setEdges((es) => es.filter((e) => e.source !== selectedNode && e.target !== selectedNode))
      setSelectedNode(null)
    } else if (selectedEdge) {
      setEdges((es) => es.filter((e) => e.id !== selectedEdge))
      setSelectedEdge(null)
    }
  }

  return (
    <div className="editor">
      <div className="canvas">
        <div className="editor-toolbar">
          <button className="secondary small" onClick={onBack}>
            ← Back
          </button>
          <input
            className="wf-name"
            style={{ width: 200 }}
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
          <span className="dim" title="Loop budget: max agent activations per run">
            max steps
          </span>
          <input
            type="number"
            min={1}
            max={500}
            style={{ width: 64 }}
            value={maxSteps}
            onChange={(e) => setMaxSteps(Number(e.target.value) || 25)}
          />
          <button onClick={save}>{saved ? '✓ Saved' : 'Save'}</button>
        </div>
        <ReactFlow
          nodes={nodes}
          edges={edges}
          nodeTypes={nodeTypes}
          onNodesChange={onNodesChange}
          onEdgesChange={onEdgesChange}
          onConnect={onConnect}
          onNodeClick={(_, n) => {
            setSelectedNode(n.id)
            setSelectedEdge(null)
          }}
          onEdgeClick={(_, e) => {
            setSelectedEdge(e.id)
            setSelectedNode(null)
          }}
          onPaneClick={() => {
            setSelectedNode(null)
            setSelectedEdge(null)
          }}
          fitView
          colorMode="dark"
          deleteKeyCode={['Delete', 'Backspace']}
        >
          <Background />
          <Controls />
          <MiniMap pannable zoomable />
        </ReactFlow>
      </div>
      <div className="sidebar">
        {error && <div className="error-banner">{error}</div>}

        {selected && selected.data.kind === 'file' && (
          <>
            <h3>File source</h3>
            <label className="field">
              <span>File path (text file on the server machine)</span>
              <input
                value={selected.data.filePath}
                onChange={(e) => updateSelected({ filePath: e.target.value })}
                placeholder="D:\data\input.txt"
              />
            </label>
            <div className="dim" style={{ fontSize: 12, marginBottom: 10 }}>
              The file is read fresh on every run; its content is passed as
              input to the connected agent(s).
            </div>
            <button className="danger small" onClick={deleteSelected}>
              Remove node
            </button>
            <hr style={{ borderColor: 'var(--border)', margin: '16px 0' }} />
          </>
        )}

        {selected && selected.data.kind === 'file_dest' && (
          <>
            <h3>File destination</h3>
            <label className="field">
              <span>File path (written on the server machine)</span>
              <input
                value={selected.data.filePath}
                onChange={(e) => updateSelected({ filePath: e.target.value })}
                placeholder="D:\data\result.md"
              />
            </label>
            <div className="checkbox-row">
              <input
                type="checkbox"
                id="dest-append"
                checked={selected.data.append ?? false}
                onChange={(e) => updateSelected({ append: e.target.checked })}
              />
              <label htmlFor="dest-append">Append instead of overwrite</label>
            </div>
            <div className="dim" style={{ fontSize: 12, marginBottom: 10 }}>
              The input this node receives is written to the file on every
              activation (missing folders are created). It also passes the
              content through unchanged, so you can chain further agents
              after it.
            </div>
            <button className="danger small" onClick={deleteSelected}>
              Remove node
            </button>
            <hr style={{ borderColor: 'var(--border)', margin: '16px 0' }} />
          </>
        )}

        {selected && selected.data.kind !== 'file' && selected.data.kind !== 'file_dest' && (
          <>
            <h3>Node settings</h3>
            <label className="field">
              <span>Agent card</span>
              <select
                value={selected.data.agentCardId}
                onChange={(e) => updateSelected({ agentCardId: e.target.value })}
              >
                {agents.map((a) => (
                  <option key={a.id} value={a.id}>
                    {a.name}
                  </option>
                ))}
              </select>
            </label>
            <label className="field">
              <span>Task in this workflow (appended to system prompt)</span>
              <textarea
                rows={5}
                value={selected.data.instructions}
                onChange={(e) => updateSelected({ instructions: e.target.value })}
                placeholder="What exactly should this agent do here?"
              />
            </label>
            <button className="danger small" onClick={deleteSelected}>
              Remove node
            </button>
            <hr style={{ borderColor: 'var(--border)', margin: '16px 0' }} />
          </>
        )}

        {selEdge && (
          <>
            <h3>
              Edge: {nodeName(selEdge.source)} → {nodeName(selEdge.target)}
            </h3>
            <label className="field">
              <span>Condition</span>
              <select
                value={selEdgeData.conditionKind ?? 'always'}
                onChange={(e) =>
                  updateSelectedEdge({ conditionKind: e.target.value as ConditionKind })
                }
              >
                <option value="always">always (unconditional)</option>
                <option value="contains">output contains text</option>
                <option value="regex">output matches regex</option>
                <option value="llm">LLM decides (natural language)</option>
              </select>
            </label>
            {selEdgeData.conditionKind !== 'always' && (
              <>
                <label className="field">
                  <span>
                    {selEdgeData.conditionKind === 'contains'
                      ? 'Text (case-insensitive)'
                      : selEdgeData.conditionKind === 'regex'
                        ? 'Regular expression'
                        : 'Condition, e.g. "the review approves the draft"'}
                  </span>
                  <textarea
                    rows={2}
                    value={selEdgeData.condition ?? ''}
                    onChange={(e) => updateSelectedEdge({ condition: e.target.value })}
                  />
                </label>
                <div className="checkbox-row">
                  <input
                    type="checkbox"
                    id="edge-negate"
                    checked={selEdgeData.negate ?? false}
                    onChange={(e) => updateSelectedEdge({ negate: e.target.checked })}
                  />
                  <label htmlFor="edge-negate">Negate (fire when condition is NOT met)</label>
                </div>
              </>
            )}
            <div className="dim" style={{ fontSize: 12, marginBottom: 10 }}>
              Pointing an edge back to an earlier agent creates a loop; the run
              stops when the condition stops firing or max steps is reached.
            </div>
            <button className="danger small" onClick={deleteSelected}>
              Remove edge
            </button>
            <hr style={{ borderColor: 'var(--border)', margin: '16px 0' }} />
          </>
        )}

        {!selected && !selEdge && (
          <>
            <h3>Agents — click to add</h3>
            {agents.length === 0 && (
              <div className="dim">No agent cards yet — create some in the Agents tab.</div>
            )}
            {agents.map((a) => (
              <div key={a.id} className="palette-item">
                <div>
                  <div className="name">{a.name}</div>
                  <div className="desc">{a.description}</div>
                </div>
                <button className="small" onClick={() => addAgentNode(a)}>
                  + Add
                </button>
              </div>
            ))}
            <h3 style={{ marginTop: 14 }}>Sources & destinations</h3>
            <div className="palette-item">
              <div>
                <div className="name">📄 File source</div>
                <div className="desc">Feeds the content of a text file into the workflow.</div>
              </div>
              <button className="small" onClick={addFileNode}>
                + Add
              </button>
            </div>
            <div className="palette-item">
              <div>
                <div className="name">💾 File destination</div>
                <div className="desc">Writes the output it receives to a file (overwrite or append).</div>
              </div>
              <button className="small" onClick={addFileDestNode}>
                + Add
              </button>
            </div>
            <div className="dim" style={{ fontSize: 12, marginTop: 6 }}>
              Tip: click an edge to give it a condition, or draw an edge back to
              an earlier agent to build a loop.
            </div>
            <hr style={{ borderColor: 'var(--border)', margin: '16px 0' }} />
          </>
        )}

        <h3>Run</h3>
        <label className="field">
          <span>Workflow input (optional, goes to source agents)</span>
          <textarea rows={3} value={runInput} onChange={(e) => setRunInput(e.target.value)} />
        </label>
        <div className="row">
          <button onClick={run}>▶ Run now</button>
          {activeRunId && (
            <>
              <button className="secondary small" onClick={() => onOpenRun(activeRunId)}>
                Open run details
              </button>
              <button className="danger small" onClick={() => api.cancelRun(activeRunId)}>
                Cancel
              </button>
            </>
          )}
        </div>
        {runLog.length > 0 && (
          <div className="run-log" ref={logRef} style={{ marginTop: 10 }}>
            {runLog.map((ev, i) => (
              <div key={i} className="ev">
                <span className="k">{ev.kind}</span>{' '}
                {ev.kind === 'tool_call' && `${ev.data.tool}(${JSON.stringify(ev.data.arguments)})`}
                {ev.kind === 'node_started' &&
                  `${ev.data.agent_name}${ev.data.activation > 1 ? ` (pass ${ev.data.activation})` : ''}`}
                {ev.kind === 'node_skipped' && ev.data.agent_name}
                {ev.kind === 'edge_resolved' && (ev.data.taken ? 'taken' : 'not taken')}
                {ev.kind === 'node_finished' &&
                  `${ev.data.status}${ev.data.error ? `: ${ev.data.error}` : ''}`}
                {ev.kind === 'run_finished' &&
                  `${ev.data.status}${ev.data.error ? `: ${ev.data.error}` : ''}`}
                {ev.kind === 'agent_step' && `${ev.data.tool_calls} tool call(s) requested`}
              </div>
            ))}
          </div>
        )}

        <hr style={{ borderColor: 'var(--border)', margin: '16px 0' }} />
        <h3>Schedules</h3>
        {schedules.map((s) => (
          <div key={s.id} className="card">
            <div className="row between">
              <code>{s.cron}</code>
              <div className="row">
                <button className="secondary small" onClick={() => toggleSchedule(s)}>
                  {s.enabled ? 'Disable' : 'Enable'}
                </button>
                <button className="danger small" onClick={() => deleteSchedule(s.id)}>
                  ✕
                </button>
              </div>
            </div>
            <div className="dim">
              {s.enabled ? 'enabled' : 'disabled'}
              {s.last_run_at && ` · last run ${new Date(s.last_run_at).toLocaleString()}`}
            </div>
          </div>
        ))}
        <label className="field">
          <span>Cron (5 or 6 fields, e.g. "0 8 * * *" = daily 08:00 UTC)</span>
          <input value={newCron} onChange={(e) => setNewCron(e.target.value)} />
        </label>
        <label className="field">
          <span>Input for scheduled runs (optional)</span>
          <input value={newCronInput} onChange={(e) => setNewCronInput(e.target.value)} />
        </label>
        <button className="secondary small" onClick={addSchedule}>
          + Add schedule
        </button>
      </div>
    </div>
  )
}
