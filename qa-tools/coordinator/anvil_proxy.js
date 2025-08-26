// anvil_proxy.js
// JSON-RPC HTTP+WS proxy that HOLDS a QA config override (ws://127.0.0.1:<ALT_PORT>)
// until the process stops, then reliably restores the original file.

const http = require('http');
const fetch = require('node-fetch');
const { URL } = require('url');
const WebSocket = require('ws');
const fs = require('fs');
const path = require('path');

// ---------- HOLDING CONFIG OVERRIDE ----------
const ORIGINAL_PORT = String(process.env.ORIGINAL_PORT || '8545'); // port currently in YAML
const ALT_PORT = String(process.env.ALT_PORT || '8546');           // proxy listen + YAML override
const PORT = parseInt(ALT_PORT, 10);
const TARGET_HOST = process.env.TARGET_HOST || 'localhost';        // upstream host
const TARGET_PORT = parseInt(process.env.TARGET_PORT || '8545', 10); // upstream port (anvil)
const CONFIG_FILE = process.env.CONFIG_FILE
    || path.resolve(__dirname, '../../config/qa/common.yaml');
const BACKUP_FILE = `${CONFIG_FILE}.backup`;
const LOCK_FILE = process.env.LOCK_FILE || '/tmp/anvil_proxy_hold.lock';

const exists = p => { try { fs.accessSync(p); return true; } catch { return false; } };

let restored = false;

function forceSwapToOriginalIfNeeded() {
    // Fallback: if no backup, rewrite ALT→ORIGINAL in-place.
    try {
        if (!exists(CONFIG_FILE)) return;
        const src = fs.readFileSync(CONFIG_FILE, 'utf8');
        const reAlt = new RegExp(
            String.raw`(^\s*url:\s*")(ws|http):\/\/(localhost|127\.0\.0\.1):${ALT_PORT}(")`,
            'm'
        );
        if (reAlt.test(src)) {
            const fixed = src.replace(reAlt, (_m, pre, scheme, host, post) =>
                `${pre}${scheme}://${host}:${ORIGINAL_PORT}${post}`
            );
            fs.writeFileSync(CONFIG_FILE, fixed, 'utf8');
        }
    } catch {}
}

function verifyRestored() {
    try {
        const now = fs.readFileSync(CONFIG_FILE, 'utf8');
        const reOk = new RegExp(
            String.raw`^\s*url:\s*"(ws|http):\/\/(localhost|127\.0\.0\.1):` + ORIGINAL_PORT + String.raw`"`,
            'm'
        );
        if (!reOk.test(now)) {
            // As a final guard, force-replace any ALT occurrences.
            forceSwapToOriginalIfNeeded();
        }
    } catch {}
}

function restoreConfigOnce() {
    if (restored) return;
    restored = true;
    try {
        if (exists(BACKUP_FILE)) {
            // Write the backup content back (more robust than rename across FS boundaries).
            const buf = fs.readFileSync(BACKUP_FILE);
            fs.writeFileSync(CONFIG_FILE, buf);
            fs.unlinkSync(BACKUP_FILE);
        } else {
            // No backup present; attempt a direct swap back.
            forceSwapToOriginalIfNeeded();
        }
    } catch {}
    try { if (exists(LOCK_FILE)) fs.unlinkSync(LOCK_FILE); } catch {}
    verifyRestored();
}

function patchConfigOrExit() {
    if (!exists(CONFIG_FILE)) {
        console.error(`config not found: ${CONFIG_FILE}`);
        process.exit(1);
    }
    if (exists(LOCK_FILE)) {
        console.error(`already running (lock: ${LOCK_FILE})`);
        process.exit(1);
    }
    if (exists(BACKUP_FILE)) {
        console.error(`backup exists: ${BACKUP_FILE} (stale state)`);
        process.exit(1);
    }

    const src = fs.readFileSync(CONFIG_FILE, 'utf8');

    // Match:   url: "ws://127.0.0.1:8545"
    // Tolerate ws/http and localhost/127.0.0.1; preserve scheme and host.
    const re = new RegExp(
        String.raw`(^\s*url:\s*")(ws|http):\/\/(localhost|127\.0\.0\.1):${ORIGINAL_PORT}(")`,
        'm'
    );

    if (!re.test(src)) {
        console.error(
            `No match for: url: "(ws|http)://(localhost|127.0.0.1):${ORIGINAL_PORT}" in ${CONFIG_FILE}`
        );
        process.exit(1);
    }

    const replaced = src.replace(re, (_m, pre, scheme, host, post) =>
        `${pre}${scheme}://${host}:${ALT_PORT}${post}`
    );

    // Backup and write atomically
    fs.writeFileSync(BACKUP_FILE, src, 'utf8');
    fs.writeFileSync(CONFIG_FILE, replaced, 'utf8');

    fs.writeFileSync(
        LOCK_FILE,
        `pid=${process.pid}\nfile=${CONFIG_FILE}\nalt_port=${ALT_PORT}\ntime=${new Date().toISOString()}\n`,
        'utf8'
    );

    const line = replaced.split('\n').find(l => l.trim().startsWith('url:'));
    if (line) console.log(`cfg: ${line.trim()}`);
}

// Restore on any exit path.
process.on('SIGINT', () => { restoreConfigOnce(); process.exit(0); });
process.on('SIGTERM', () => { restoreConfigOnce(); process.exit(0); });
process.on('uncaughtException', (e) => { console.error(e); restoreConfigOnce(); process.exit(1); });
process.on('exit', () => { restoreConfigOnce(); });

// Apply config patch BEFORE starting the proxy.
patchConfigOrExit();

// ---------- DIFFICULTY INJECTION ----------
// difficulty = 10000 + blockNumber
// totalDifficulty = sum_{k=0..blockNumber}(10000 + k)
function injectDifficulty(msgObj) {
    if (!msgObj || !msgObj.result || msgObj.result.number === undefined) return;
    const bn = BigInt(msgObj.result.number);
    const diff = 10000n + bn;
    msgObj.result.difficulty = '0x' + diff.toString(16);
    const total = (bn + 1n) * 10000n + (bn * (bn + 1n) / 2n);
    msgObj.result.totalDifficulty = '0x' + total.toString(16);
}

// ---------- HTTP JSON-RPC PROXY ----------
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
            try { rpcMethod = body ? JSON.parse(body).method : undefined; } catch {}
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

// ---------- WS PROXY ----------
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
            } else if (
                msgObj.method === 'eth_subscription' &&
                msgObj.params && msgObj.params.result &&
                msgObj.params.result.number !== undefined
            ) {
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

server.on('upgrade', (req, socket, head) => {
    wsServer.handleUpgrade(req, socket, head, ws => wsServer.emit('connection', ws, req));
});

server.listen(PORT, () => {
    console.log(
        `Anvil proxy on ${PORT} (holding QA config to ws://127.0.0.1:${PORT}) -> upstream ${TARGET_HOST}:${TARGET_PORT}`
    );
});