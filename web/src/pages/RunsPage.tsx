import { useEffect, useState } from 'react'
import { api, subscribeRunEvents } from '../api'
import type { NodeRun, Run } from '../types'

function ts(s?: string | null) {
  if (!s) return ''
  return new Date(s).toLocaleString()
}

function duration(run: Run) {
  if (!run.finished_at) return '…'
  const ms = new Date(run.finished_at).getTime() - new Date(run.started_at).getTime()
  return ms < 1000 ? `${ms}ms` : `${(ms / 1000).toFixed(1)}s`
}

export default function RunsPage({
  viewRunId,
  onViewRun,
}: {
  viewRunId: string | null
  onViewRun: (id: string | null) => void
}) {
  const [runs, setRuns] = useState<Run[]>([])
  const [detail, setDetail] = useState<{ run: Run; node_runs: NodeRun[] } | null>(null)
  const [error, setError] = useState('')

  const reloadList = () => api.runs().then(setRuns).catch((e) => setError(e.message))
  const reloadDetail = (id: string) =>
    api.run(id).then(setDetail).catch((e) => setError(e.message))

  useEffect(() => {
    reloadList()
  }, [])

  useEffect(() => {
    if (viewRunId) reloadDetail(viewRunId)
    else setDetail(null)
  }, [viewRunId])

  // Refresh on live events for the run being viewed (and the list on finishes).
  useEffect(() => {
    return subscribeRunEvents((ev) => {
      if (ev.kind === 'run_started' || ev.kind === 'run_finished') reloadList()
      if (viewRunId && ev.run_id === viewRunId) reloadDetail(viewRunId)
    })
  }, [viewRunId])

  const cancel = async (id: string) => {
    try {
      await api.cancelRun(id)
    } catch (e: any) {
      setError(e.message)
    }
  }

  return (
    <div className="page">
      {error && <div className="error-banner">{error}</div>}
      <div className="split">
        <div className="list-col">
          <div className="row between" style={{ marginBottom: 12 }}>
            <h2 style={{ margin: 0 }}>Runs</h2>
            <button className="secondary small" onClick={reloadList}>
              Refresh
            </button>
          </div>
          {runs.map((r) => (
            <div
              key={r.id}
              className={`card clickable ${viewRunId === r.id ? 'selected' : ''}`}
              onClick={() => onViewRun(r.id)}
            >
              <div className="row between">
                <h3>{r.workflow_name}</h3>
                <span className={`badge ${r.status}`}>{r.status}</span>
              </div>
              <div className="dim">
                {ts(r.started_at)} · {duration(r)} · {r.trigger}
              </div>
            </div>
          ))}
          {runs.length === 0 && <div className="dim card">No runs yet.</div>}
        </div>
        <div className="detail-col">
          {detail ? (
            <>
              <div className="card">
                <div className="row between">
                  <h3>
                    {detail.run.workflow_name}{' '}
                    <span className={`badge ${detail.run.status}`}>{detail.run.status}</span>
                  </h3>
                  {detail.run.status === 'running' && (
                    <button className="danger small" onClick={() => cancel(detail.run.id)}>
                      Cancel
                    </button>
                  )}
                </div>
                <div className="dim">
                  started {ts(detail.run.started_at)}
                  {detail.run.finished_at && ` · finished ${ts(detail.run.finished_at)}`}
                </div>
                {detail.run.input && (
                  <details>
                    <summary>Workflow input</summary>
                    <pre className="output">{detail.run.input}</pre>
                  </details>
                )}
                {detail.run.error && <div className="error-banner">{detail.run.error}</div>}
              </div>
              {detail.node_runs.map((nr) => (
                <div key={nr.id} className="card">
                  <div className="row between">
                    <h3>
                      {nr.agent_name}
                      {nr.activation > 1 && (
                        <span className="dim" style={{ fontWeight: 400 }}>
                          {' '}
                          · pass {nr.activation}
                        </span>
                      )}
                    </h3>
                    <span className={`badge ${nr.status}`}>{nr.status}</span>
                  </div>
                  {nr.error && <div className="error-banner">{nr.error}</div>}
                  <details>
                    <summary>Input</summary>
                    <pre className="output">{nr.input}</pre>
                  </details>
                  {nr.output && (
                    <details open={nr.status === 'succeeded'}>
                      <summary>Output</summary>
                      <pre className="output">{nr.output}</pre>
                    </details>
                  )}
                  {Array.isArray(nr.transcript) && (
                    <details>
                      <summary>Full transcript ({(nr.transcript as any[]).length} messages)</summary>
                      <pre className="output">{JSON.stringify(nr.transcript, null, 2)}</pre>
                    </details>
                  )}
                </div>
              ))}
              {detail.node_runs.length === 0 && (
                <div className="dim card">No agent has started yet.</div>
              )}
            </>
          ) : (
            <div className="dim" style={{ padding: 30 }}>
              Select a run to inspect every agent's input, output and tool calls.
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
