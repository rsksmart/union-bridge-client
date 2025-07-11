// anvil_proxy.js
const http = require('http');
const fetch = require('node-fetch');
const { URL } = require('url');
const WebSocket = require('ws');

// Proxy configuration
const PORT = 8546;
const TARGET_HOST = 'localhost';
const TARGET_PORT = 8545;

// Simplified difficulty injection:
// difficulty = 10000 + blockNumber
// totalDifficulty = sum_{k=0..blockNumber}(10000 + k)
function injectDifficulty(msgObj) {
    if (!msgObj || !msgObj.result || msgObj.result.number === undefined) return;
    const bn = BigInt(msgObj.result.number);
    // per-block difficulty
    const diff = 10000n + bn;
    msgObj.result.difficulty = '0x' + diff.toString(16);
    // totalDifficulty = (bn + 1) * 10000 + (bn * (bn + 1) / 2)
    const total = (bn + 1n) * 10000n + (bn * (bn + 1n) / 2n);
    msgObj.result.totalDifficulty = '0x' + total.toString(16);
}

// HTTP JSON-RPC proxy
const server = http.createServer(async (req, res) => {
    try {
        let originalUrl = req.url;
        if (originalUrl.startsWith('http://')) {
            const parsed = new URL(originalUrl);
            originalUrl = parsed.pathname + parsed.search;
        }
        const targetUrl = `http://${TARGET_HOST}:${TARGET_PORT}${originalUrl}`;
        const headers = { ...req.headers, host: `${TARGET_HOST}:${TARGET_PORT}` };
        let body = null;
        if (req.method !== 'GET' && req.method !== 'HEAD') {
            body = await new Promise((resolve, reject) => {
                let data = '';
                req.on('data', chunk => data += chunk);
                req.on('end', () => resolve(data));
                req.on('error', reject);
            });
        }
        const upstream = await fetch(targetUrl, { method: req.method, headers, body });
        const forwardHeaders = Object.fromEntries(upstream.headers.entries());
        delete forwardHeaders['content-length'];
        res.writeHead(upstream.status, forwardHeaders);
        const ct = upstream.headers.get('content-type') || '';
        if (ct.includes('application/json')) {
            const data = await upstream.json();
            let rpcMethod;
            try { rpcMethod = JSON.parse(body).method; } catch {}
            if (rpcMethod === 'eth_getBlockByNumber' || rpcMethod === 'eth_getBlockByHash') {
                injectDifficulty(data);
            }
            return res.end(JSON.stringify(data));
        }
        const text = await upstream.text();
        return res.end(text);
    } catch (err) {
        res.writeHead(500, { 'Content-Type': 'text/plain' });
        res.end(String(err));
    }
});

// WebSocket proxy for subscriptions
const wsServer = new WebSocket.Server({ noServer: true });
wsServer.on('connection', (clientWs, req) => {
    const targetWs = new WebSocket(`ws://${TARGET_HOST}:${TARGET_PORT}${req.url}`);
    clientWs.on('message', msg => targetWs.send(msg));
    targetWs.on('message', raw => {
        let payload = raw.toString();
        try {
            const msgObj = JSON.parse(payload);
            if (msgObj.id && msgObj.result && msgObj.result.number !== undefined) {
                injectDifficulty(msgObj);
                payload = JSON.stringify(msgObj);
            } else if (msgObj.method === 'eth_subscription' && msgObj.params && msgObj.params.result && msgObj.params.result.number !== undefined) {
                injectDifficulty(msgObj.params);
                payload = JSON.stringify(msgObj);
            }
        } catch {}
        clientWs.send(payload);
    });
    clientWs.on('close', (code, reason) => targetWs.close(code, reason));
    clientWs.on('error', () => targetWs.terminate());
    targetWs.on('close', (code, reason) => clientWs.close(code, reason));
    targetWs.on('error', () => clientWs.terminate());
});

// Handle HTTP->WS upgrades
server.on('upgrade', (req, socket, head) => {
    wsServer.handleUpgrade(req, socket, head, ws => wsServer.emit('connection', ws, req));
});

server.listen(PORT, () => console.log(`Anvil proxy on ${PORT} forwarding to ${TARGET_HOST}:${TARGET_PORT}`));