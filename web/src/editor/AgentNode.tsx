import { Handle, Position, type NodeProps } from '@xyflow/react'

export type AgentNodeData = {
  kind: 'agent' | 'file' | 'file_dest'
  label: string
  agentCardId: string
  instructions: string
  filePath: string
  append?: boolean
  status?: string
  activation?: number
}

export default function AgentNode({ data, selected }: NodeProps) {
  const d = data as AgentNodeData
  const status = d.status ? `status-${d.status}` : ''
  return (
    <div className={`agent-node ${status} ${selected ? 'selected-node' : ''}`}>
      <Handle type="target" position={Position.Left} />
      <div className="title">{d.label}</div>
      {d.instructions && (
        <div className="sub">
          {d.instructions.length > 70 ? d.instructions.slice(0, 70) + '…' : d.instructions}
        </div>
      )}
      {d.status && (
        <div className="sub">
          {d.status}
          {(d.activation ?? 0) > 1 ? ` · pass ${d.activation}` : ''}
        </div>
      )}
      <Handle type="source" position={Position.Right} />
    </div>
  )
}
