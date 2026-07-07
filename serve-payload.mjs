import { createServer } from 'node:http'
import { readFileSync } from 'node:fs'

const payload = readFileSync(new URL('./ct-payload.json', import.meta.url))
const port = parseInt(process.argv[2] || '8080', 10)

createServer((_req, res) => {
  res.writeHead(200, {
    'content-type': 'application/json',
    'content-length': payload.length,
    'access-control-allow-origin': '*',
  })
  res.end(payload)
}).listen(port, () => {
  console.error(`listening on http://localhost:${port}`)
})
