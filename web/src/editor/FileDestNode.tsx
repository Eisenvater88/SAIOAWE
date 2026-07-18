import { Handle, Position, type NodeProps } from '@xyflow/react'
import type { AgentNodeData } from './AgentNode'

export default function FileDestNode({ data, selected }: NodeProps) {
  const d = data as AgentNodeData
  const status = d.status ? `status-${d.status}` : ''
  return (
    <div className={`agent-node file-node ${status} ${selected ? 'selected-node' : ''}`}>
      <Handle type="target" position={Position.Left} />
      <div className="title">💾 {d.label}</div>
      {d.filePath && (
        <div className="sub">
          {d.filePath.length > 40 ? '…' + d.filePath.slice(-40) : d.filePath}
          {d.append ? ' (append)' : ''}
        </div>
      )}
      {d.status && <div className="sub">{d.status}</div>}
      <Handle type="source" position={Position.Right} />
    </div>
  )
}
