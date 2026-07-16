export interface AgentCard {
  id: string
  name: string
  description: string
  model?: string | null
  system_prompt: string
  mcp_servers: string[]
  temperature?: number | null
  max_tool_iterations: number
  created_at?: string
  updated_at?: string
}

export interface Position {
  x: number
  y: number
}

export interface WorkflowNode {
  id: string
  agent_card_id: string
  instructions: string
  position: Position
}

export type ConditionKind = 'always' | 'contains' | 'regex' | 'llm'

export interface WorkflowEdge {
  id: string
  source: string
  target: string
  condition_kind: ConditionKind
  condition: string
  negate: boolean
}

export interface Graph {
  nodes: WorkflowNode[]
  edges: WorkflowEdge[]
}

export interface Workflow {
  id: string
  name: string
  description: string
  graph: Graph
  max_steps: number
  created_at?: string
  updated_at?: string
}

export interface McpServerConfig {
  id: string
  name: string
  transport: 'stdio' | 'http'
  command: string
  args: string[]
  env: Record<string, string>
  url: string
  headers: Record<string, string>
  enabled: boolean
  created_at?: string
  updated_at?: string
}

export interface McpTool {
  name: string
  description: string
  input_schema: unknown
}

export interface Schedule {
  id: string
  workflow_id: string
  cron: string
  input: string
  enabled: boolean
  last_run_at?: string | null
  created_at?: string
}

export interface Run {
  id: string
  workflow_id: string
  workflow_name: string
  status: 'pending' | 'running' | 'succeeded' | 'failed' | 'canceled' | 'interrupted'
  trigger: string
  input: string
  error?: string | null
  started_at: string
  finished_at?: string | null
}

export interface NodeRun {
  id: string
  run_id: string
  node_id: string
  agent_name: string
  status: string
  activation: number
  input: string
  output: string
  transcript: unknown
  error?: string | null
  started_at: string
  finished_at?: string | null
}

export interface RunEvent {
  run_id: string
  workflow_id: string
  node_id?: string
  kind: string
  data: any
  ts: string
}

export interface ServerConfig {
  ollama_url: string
  default_model: string
  default_temperature: number
  models: string[]
}
