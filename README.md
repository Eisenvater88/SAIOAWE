# SAIOAWE — Agent Workflow Orchestrator

A self-hosted agent runtime: describe agents with **agent cards**, wire them
into **directed workflows** on a visual canvas, give them tools via **MCP
servers**, and run everything **on demand or on a cron schedule** — powered by
a local **Ollama** instance.

## Architecture

```
┌────────────────────────── Rust backend (axum) ──────────────────────────┐
│  clap config      SQLite store        workflow engine       scheduler   │
│  (CLI + env)      (cards, flows,      (parallel DAG          (cron,     │
│                    runs, sched.)       agent loops)           15s tick) │
│                                             │                           │
│                     ┌───────────────────────┼──────────────────┐        │
│                     ▼                       ▼                  ▼        │
│               Ollama /api/chat        MCP client          SSE events    │
│               (tool calling)          (stdio + http)      (live UI)     │
└──────────────────────────────────────────────────────────────────────────┘
                                    ▲
                    React + React Flow web UI (web/)
```

- **Agent card** = name, system prompt, model (optional, falls back to server
  default), allowed MCP servers, temperature, tool budget. Cards are yours to
  author — the orchestration doesn't care what the agents do.
- **Workflow** = agent nodes + directed edges. Each edge feeds the source
  agent's final output into the target agent's input. Fan-out (one output to
  many agents) and fan-in (many outputs joined into one input) both work;
  independent branches run in parallel.
- **Conditional edges**: an edge can require the source output to
  *contain* a text, *match* a regex, or satisfy a natural-language predicate
  judged by the LLM ("the review approves the draft") — each optionally
  negated for else-branches. Agents whose incoming edges all stay silent are
  skipped (skips cascade); a fan-in agent runs with whichever inputs fired.
- **Loops**: draw an edge back to an earlier agent and it becomes a loop —
  each firing re-activates that agent with the new payload (classic pattern:
  drafter → reviewer, with a "not approved" edge back to the drafter). Every
  activation consumes one step of the workflow's **max steps** budget
  (default 25); exceeding it fails the run instead of looping forever.
- **Run** = every node executes an LLM loop: Ollama replies either with tool
  calls (dispatched to MCP servers, results fed back) or with a final answer
  that travels along the outgoing edges. Full transcripts are persisted.

## Build & run

```powershell
# backend
cargo build --release

# frontend
cd web
npm install
npm run build
cd ..

# start (defaults shown)
.\target\release\saioawe.exe --host 127.0.0.1 --port 8321 `
  --ollama-url http://127.0.0.1:11434 --ollama-model llama3.1 `
  --db saioawe.db --web-dir web/dist
```

Open http://127.0.0.1:8321. Every flag is also an env var (`SAIOAWE_HOST`,
`SAIOAWE_PORT`, `OLLAMA_URL`, `OLLAMA_MODEL`, `OLLAMA_TEMPERATURE`,
`SAIOAWE_DB`, `SAIOAWE_WEB_DIR`, `SAIOAWE_LLM_TIMEOUT`, `SAIOAWE_TOOL_TIMEOUT`,
`SAIOAWE_ALLOWED_ORIGINS`).
Use a tool-capable model (e.g. `gemma4:latest`, `qwen3.5:9b`) — agents without
MCP servers work with any model.

For UI development: `cd web && npm run dev` (Vite proxies `/api` to :8321 and
strips the browser Origin so the same-origin guard is satisfied).

## Security model

The API is unauthenticated and can spawn user-configured MCP commands, so it
must never be reachable by a hostile web page. A same-origin guard enforces
this: for a loopback bind the `Host` header must be loopback (defeats DNS
rebinding), and any `Origin` header must match the request's own host (blocks
cross-site `fetch()` — the drive-by RCE vector). Header-less clients (curl,
scripts) and the served UI pass untouched.

Bind to loopback (`127.0.0.1`, the default) for personal use. To expose it
beyond localhost, put it behind a reverse proxy that adds real authentication —
`--host 0.0.0.0` alone has no auth. Add extra allowed browser origins (a dev
server, a proxy) with `--allowed-origin http://localhost:5173` (repeatable).

## Using it

1. **MCP Servers tab** — register the servers your agents need.
   - *stdio*: command + args (+ env), e.g. `npx -y @some/mcp-server`.
     Windows batch shims (`npx`, `uvx`) are handled automatically.
   - *http*: streamable-HTTP endpoint URL (+ headers, e.g. Authorization).
   - "Test connection" spawns/connects and lists the server's tools.
2. **Agents tab** — write your agent cards and tick the MCP servers each
   agent may use.
3. **Workflows tab** — create a workflow, click agents into the canvas,
   drag edges between them. Select a node to give it workflow-specific
   instructions. Save, then **Run now** (with optional input) and watch the
   nodes light up live; or add a **schedule** (5- or 6-field cron, UTC).
4. **Runs tab** — history with per-agent input, output, error and the full
   message/tool transcript.

## Example workflow (the anime pipeline)

`examples/mock-crunchyroll-mcp.js` is a tiny stdio MCP server (plain Node,
no dependencies) that fakes a Crunchyroll watch-history tool — register it
with command `node` and the file path as argument. The intended real
pipeline looks like:

```
[Crunchyroll agent] ──▶ [Web research agent] ──┬──▶ [E-mail agent]
 (MCP: crunchyroll)      (MCP: web search)     └──▶ [Calendar agent]
```

Agents 3 and 4 both receive agent 2's output (fan-out). Swap the mock for a
real Crunchyroll MCP server plus e-mail/calendar MCP servers and the same
graph runs unchanged.

## API (for scripting)

`GET/POST /api/agents`, `/api/workflows`, `/api/mcp-servers`, `/api/schedules`
(+ `GET/PUT/DELETE .../{id}`), `POST /api/workflows/{id}/run {input}`,
`GET /api/runs[?workflow_id=]`, `GET /api/runs/{id}`, `POST /api/runs/{id}/cancel`,
`GET /api/mcp-servers/{id}/tools`, `GET /api/events` (SSE), `GET /api/config`.

## Notes & current limits

- Loop semantics: after its first activation, each firing edge re-activates
  the target with that payload alone (loop bodies are best kept linear).
  A conditional edge leaving a loop stays pending until it fires — when the run
  goes quiescent, still-pending edges settle as silent, which can complete a
  waiting join node (or mark truly unreached nodes "skipped").
- A failing node fails the run (in-flight sibling branches finish first).
- Crash safety: runs execute in memory, so if the process dies mid-run the run
  cannot continue. At startup any run left `running` is marked `interrupted`
  (not auto-resumed — a node may already have sent an e-mail or created a
  calendar entry, and re-running would repeat it). Re-trigger it manually.
- MCP: tools only (no resources/prompts/sampling yet); server-initiated
  requests are ignored.
- Scheduler granularity is ~15 s; missed fires while the server is down
  collapse into one run.
