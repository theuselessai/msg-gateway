"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
const crypto_1 = require("crypto");
const fastify_1 = __importDefault(require("fastify"));
const eventsource_client_1 = require("eventsource-client");
const INSTANCE_ID = process.env.INSTANCE_ID ?? "unknown";
const BACKEND_PORT = parseInt(process.env.BACKEND_PORT ?? "9200", 10);
const GATEWAY_URL = process.env.GATEWAY_URL ?? "http://localhost:8080";
const BACKEND_TOKEN = process.env.BACKEND_TOKEN ?? "";
const GATEWAY_SEND_TOKEN = process.env.GATEWAY_SEND_TOKEN ?? "";
const MAX_SESSIONS = 10000;
const backendConfig = (() => {
    try {
        return JSON.parse(process.env.BACKEND_CONFIG ?? "{}");
    }
    catch {
        return { base_url: "", token: "" };
    }
})();
function log(msg) {
    const ts = new Date().toISOString();
    process.stderr.write(`[${ts}] [${INSTANCE_ID}] ${msg}\n`);
}
function logError(prefix, err) {
    const msg = err instanceof Error ? err.message : String(err);
    log(`${prefix}: ${msg}`);
    if (err instanceof Error && err.stack) {
        log(err.stack);
    }
}
function verifyBearer(header, token) {
    if (!token)
        return false;
    const expectedBuf = Buffer.from(`Bearer ${token}`);
    const headerBuf = Buffer.from(header);
    if (headerBuf.length !== expectedBuf.length)
        return false;
    return (0, crypto_1.timingSafeEqual)(headerBuf, expectedBuf);
}
// Session management: {credential_id}:{chat_id} -> session_id
const sessions = new Map();
// Pending sessions waiting for SSE response: sessionId -> { credentialId, chatId, timeoutHandle }
const pending = new Map();
const PENDING_TIMEOUT_MS = 10 * 60 * 1000; // 10 minutes
let shuttingDown = false;
async function retry(fn, retries = 3, baseDelay = 1000) {
    let lastError;
    for (let attempt = 0; attempt < retries; attempt++) {
        try {
            return await fn();
        }
        catch (err) {
            lastError = err;
            if (attempt < retries - 1 && !shuttingDown) {
                const delay = baseDelay * Math.pow(2, attempt);
                log(`Retry ${attempt + 1}/${retries} after ${delay}ms`);
                await new Promise((r) => setTimeout(r, delay));
            }
        }
    }
    throw lastError;
}
function parseAuth() {
    const token = backendConfig.token ?? "";
    if (!token) {
        throw new Error("BACKEND_CONFIG must include non-empty 'token'");
    }
    const colonPos = token.indexOf(":");
    if (colonPos === -1) {
        return { username: token, password: "" };
    }
    return {
        username: token.substring(0, colonPos),
        password: token.substring(colonPos + 1),
    };
}
function basicAuthHeader() {
    const { username, password } = parseAuth();
    const encoded = Buffer.from(`${username}:${password}`).toString("base64");
    return `Basic ${encoded}`;
}
async function getOrCreateSession(credentialId, chatId) {
    const sessionKey = `${credentialId}:${chatId}`;
    const existing = sessions.get(sessionKey);
    if (existing)
        return existing;
    log(`Creating new OpenCode session for ${sessionKey}`);
    const resp = await fetch(`${backendConfig.base_url}/session`, {
        method: "POST",
        headers: {
            Authorization: basicAuthHeader(),
        },
        signal: AbortSignal.timeout(30000),
    });
    if (!resp.ok) {
        const body = await resp.text();
        throw new Error(`Failed to create OpenCode session: ${resp.status} ${body}`);
    }
    const data = (await resp.json());
    const sessionId = data.id;
    // FIFO eviction: Map iterates in insertion order, so first key is earliest inserted
    if (sessions.size >= MAX_SESSIONS) {
        const firstInsertedKey = sessions.keys().next().value;
        if (firstInsertedKey !== undefined)
            sessions.delete(firstInsertedKey);
    }
    sessions.set(sessionKey, sessionId);
    log(`Session created: ${sessionId} for ${sessionKey}`);
    return sessionId;
}
async function fetchAndRelay(sessionId, credentialId, chatId) {
    const resp = await fetch(`${backendConfig.base_url}/session/${sessionId}/message`, {
        headers: { Authorization: basicAuthHeader() },
        signal: AbortSignal.timeout(30000),
    });
    if (!resp.ok) {
        log(`Failed to fetch messages for session ${sessionId}: ${resp.status}`);
        return;
    }
    const messages = await resp.json();
    // find last assistant message
    const assistantMsgs = messages.filter(m => m.info.role === "assistant");
    if (assistantMsgs.length === 0) {
        log(`No assistant message found for session ${sessionId}`);
        return;
    }
    const last = assistantMsgs[assistantMsgs.length - 1];
    const text = last.parts
        .filter(p => p.type === "text" && p.text)
        .map(p => p.text)
        .join("\n\n")
        .trim();
    if (!text) {
        log(`Empty text response for session ${sessionId}`);
        return;
    }
    log(`Relaying response to gateway for chat ${chatId}`);
    await relayToGateway(credentialId, chatId, text);
}
function handleEvent(event) {
    if (event.type === "session.idle") {
        const sessionId = event.properties.sessionID;
        const entry = pending.get(sessionId);
        if (!entry)
            return;
        clearTimeout(entry.timeoutHandle);
        pending.delete(sessionId);
        // fetch response and relay — fire and forget
        fetchAndRelay(sessionId, entry.credentialId, entry.chatId).catch(err => {
            logError(`Error relaying response for session ${sessionId}`, err);
        });
    }
    else if (event.type === "session.error") {
        const sessionId = event.properties.sessionID;
        if (sessionId) {
            const entry = pending.get(sessionId);
            if (entry) {
                clearTimeout(entry.timeoutHandle);
                pending.delete(sessionId);
                log(`Session error for ${sessionId}: ${JSON.stringify(event.properties.error)}`);
            }
        }
    }
}
function startEventStream() {
    const es = (0, eventsource_client_1.createEventSource)({
        url: `${backendConfig.base_url}/global/event`,
        headers: { Authorization: basicAuthHeader() },
        onMessage: ({ data }) => {
            if (!data)
                return;
            try {
                const globalEvent = JSON.parse(data);
                handleEvent(globalEvent.payload);
            }
            catch (err) {
                logError("SSE parse error", err);
            }
        },
        onDisconnect: () => {
            if (!shuttingDown) {
                log("SSE disconnected, will reconnect automatically");
            }
        },
    });
    return es;
}
async function relayToGateway(credentialId, chatId, text) {
    const relayBody = {
        credential_id: credentialId,
        chat_id: chatId,
        text,
    };
    await retry(async () => {
        const resp = await fetch(`${GATEWAY_URL}/api/v1/send`, {
            method: "POST",
            headers: {
                Authorization: `Bearer ${GATEWAY_SEND_TOKEN}`,
                "Content-Type": "application/json",
            },
            body: JSON.stringify(relayBody),
            signal: AbortSignal.timeout(15000),
        });
        if (!resp.ok) {
            const body = await resp.text();
            throw new Error(`Gateway relay failed: ${resp.status} ${body}`);
        }
    });
}
const app = (0, fastify_1.default)({ logger: false });
app.get("/health", async () => {
    return { status: "ok" };
});
app.post("/send", async (request, reply) => {
    const authHeader = request.headers.authorization;
    if (!authHeader || !verifyBearer(authHeader, BACKEND_TOKEN)) {
        reply.status(401);
        return { error: "Unauthorized" };
    }
    const message = request.body;
    if (!message?.credential_id || !message?.source?.chat_id || !message?.source?.from || typeof message?.text !== "string") {
        reply.status(400);
        return { error: "Invalid message: missing required fields (credential_id, source.chat_id, source.from, text)" };
    }
    const chatId = message.source.chat_id;
    const who = message.source.from.display_name ??
        message.source.from.username ??
        message.source.from.id;
    const truncatedText = message.text.length > 80 ? message.text.slice(0, 80) + "..." : message.text;
    log(`Received message from ${who} in chat ${chatId}: ${truncatedText}`);
    try {
        const sessionId = await getOrCreateSession(message.credential_id, message.source.chat_id);
        // Fire and forget: submit to OpenCode asynchronously
        const asyncResp = await fetch(`${backendConfig.base_url}/session/${sessionId}/prompt_async`, {
            method: "POST",
            headers: {
                Authorization: basicAuthHeader(),
                "Content-Type": "application/json",
            },
            body: JSON.stringify({
                model: backendConfig.model,
                parts: [{ type: "text", text: message.text }],
            }),
            signal: AbortSignal.timeout(10000),
        });
        if (!asyncResp.ok && asyncResp.status !== 204) {
            // Handle stale session same as before
            if (asyncResp.status === 401 || asyncResp.status === 403 || asyncResp.status === 404) {
                const sessionKey = `${message.credential_id}:${message.source.chat_id}`;
                sessions.delete(sessionKey);
                log(`Invalidated stale session for ${sessionKey} (HTTP ${asyncResp.status})`);
            }
            const body = await asyncResp.text();
            throw new Error(`OpenCode prompt_async failed: ${asyncResp.status} ${body}`);
        }
        // Register pending — SSE will deliver the response
        const existing = pending.get(sessionId);
        if (existing)
            clearTimeout(existing.timeoutHandle);
        const timeoutHandle = setTimeout(() => {
            pending.delete(sessionId);
            log(`Pending response timeout for session ${sessionId}`);
        }, PENDING_TIMEOUT_MS);
        pending.set(sessionId, { credentialId: message.credential_id, chatId: message.source.chat_id, timeoutHandle });
        log(`Message submitted async, waiting for SSE response (session=${sessionId})`);
        return { status: "ok" };
    }
    catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        log(`Error submitting message: ${msg}`);
        reply.status(500);
        return { error: msg };
    }
});
async function shutdown(signal, eventSource) {
    if (shuttingDown)
        return;
    shuttingDown = true;
    log(`Received ${signal}, shutting down...`);
    eventSource.close();
    try {
        await app.close();
    }
    catch (err) {
        logError("Error during shutdown", err);
    }
    log("Backend adapter stopped");
    process.exit(0);
}
async function main() {
    log("Starting OpenCode backend adapter");
    log(`  Port: ${BACKEND_PORT}`);
    log(`  Gateway: ${GATEWAY_URL}`);
    log(`  OpenCode URL: ${backendConfig.base_url}`);
    if (!BACKEND_TOKEN) {
        throw new Error("BACKEND_TOKEN environment variable must be set");
    }
    if (!backendConfig.base_url) {
        throw new Error("BACKEND_CONFIG must include 'base_url'");
    }
    if (!backendConfig.token) {
        throw new Error("BACKEND_CONFIG must include non-empty 'token'");
    }
    if (!backendConfig.model) {
        throw new Error("BACKEND_CONFIG must include 'model' with providerID and modelID");
    }
    if (!GATEWAY_SEND_TOKEN) {
        throw new Error("GATEWAY_SEND_TOKEN environment variable must be set");
    }
    const eventSource = startEventStream();
    process.on("SIGTERM", () => shutdown("SIGTERM", eventSource));
    process.on("SIGINT", () => shutdown("SIGINT", eventSource));
    await app.listen({
        port: BACKEND_PORT,
        host: process.env.BACKEND_HOST ?? "0.0.0.0",
    });
    log(`HTTP server listening on port ${BACKEND_PORT}`);
}
main().catch((err) => {
    logError("Fatal error", err);
    process.exit(1);
});
