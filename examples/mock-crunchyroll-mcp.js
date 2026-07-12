// Minimal stdio MCP server for testing SAIOAWE's MCP client.
// One tool: get_watch_history -> fake "recently watched anime" data.
const readline = require('readline')
const rl = readline.createInterface({ input: process.stdin })

const send = (obj) => process.stdout.write(JSON.stringify(obj) + '\n')

rl.on('line', (line) => {
  line = line.trim()
  if (!line) return
  let msg
  try {
    msg = JSON.parse(line)
  } catch {
    return
  }
  if (msg.method === 'initialize') {
    send({
      jsonrpc: '2.0',
      id: msg.id,
      result: {
        protocolVersion: '2024-11-05',
        capabilities: { tools: {} },
        serverInfo: { name: 'mock-crunchyroll', version: '1.0.0' },
      },
    })
  } else if (msg.method === 'tools/list') {
    send({
      jsonrpc: '2.0',
      id: msg.id,
      result: {
        tools: [
          {
            name: 'get_watch_history',
            description: 'Returns the animes the user watched recently, with genres.',
            inputSchema: {
              type: 'object',
              properties: {
                limit: { type: 'number', description: 'max entries to return' },
              },
            },
          },
        ],
      },
    })
  } else if (msg.method === 'tools/call') {
    const name = msg.params && msg.params.name
    if (name === 'get_watch_history') {
      send({
        jsonrpc: '2.0',
        id: msg.id,
        result: {
          content: [
            {
              type: 'text',
              text: JSON.stringify([
                { title: 'Frieren: Beyond Journey’s End', genre: ['Fantasy', 'Adventure'] },
                { title: 'Vinland Saga', genre: ['Action', 'Historical'] },
                { title: 'Spy x Family', genre: ['Comedy', 'Action'] },
              ]),
            },
          ],
        },
      })
    } else {
      send({
        jsonrpc: '2.0',
        id: msg.id,
        result: { content: [{ type: 'text', text: 'unknown tool' }], isError: true },
      })
    }
  } else if (msg.id !== undefined) {
    // Unknown request: reply with empty result so nothing hangs.
    send({ jsonrpc: '2.0', id: msg.id, result: {} })
  }
  // Notifications (no id) are ignored.
})
