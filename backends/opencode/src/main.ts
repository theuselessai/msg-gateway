import { timingSafeEqual } from "crypto";
import Fastify from "fastify";

const INSTANCE_ID = process.env.INSTANCE_ID ?? "unknown";
const BACKEND_PORT = parseInt(process.env.BACKEND_PORT ?? "9200", 10);
const GATEWAY_URL = process.env.GATEWAY_URL ?? "http://localhost:8080";
const BACKEND_TOKEN = process.env.BACKEND_TOKEN ?? "";
const GATEWAY_SEND_TOKEN = process.env.GATEWAY_SEND_TOKEN ?? "";

const MAX_SESSIONS = 10_000;

interface BackendConfig {
  base_url: string;
  token: string;
  model?: {
    providerID: string;
    modelID: string;
  };
}

const backendConfig: BackendConfig = (() => {
  try {
    return JSON.parse(process.env.BACKEND_CONFIG ?? "{}") as BackendConfig;
  } catch {
    return { base_url: "", token: "" };
  }
})();

function log(msg: string): void {
  const ts = new Date().toISOString();
  process.stderr.write(`[${ts}] [${INSTANCE_ID}] ${msg}\n`);
}

function verifyBearer(header: string, token: string): boolean {
  const expected = `Bearer ${token}`;
  if (header.length !== expected.length) return false;
  return timingSafeEqual(Buffer.from(header), Buffer.from(expected));
}

interface UserInfo {
  id: string;
  username?: string;
  display_name?: string;
}

interface MessageSource {
  protocol: string;
  chat_id: string;
  message_id: string;
  reply_to_message_id?: string;
  from: UserInfo;
}

interface Attachment {
  filename: string;
  mime_type: string;
  size_bytes: number;
  download_url: string;
}

interface InboundMessage {
  route: unknown;
  credential_id: string;
  source: MessageSource;
  text: string;
  attachments: Attachment[];
  timestamp: string;
  extra_data?: unknown;
}

// Session management: {credential_id}:{chat_id} -> session_id
const sessions = new Map<string, string>();

async function retry<T>(
  fn: () => Promise<T>,
  retries = 3,
  baseDelay = 1000,
): Promise<T> {
  let lastError: unknown;
  for (let attempt = 0; attempt < retries; attempt++) {
    try {
      return await fn();
    } catch (err) {
      lastError = err;
      if (attempt < retries - 1) {
        const delay = baseDelay * Math.pow(2, attempt);
        log(`Retry ${attempt + 1}/${retries} after ${delay}ms`);
        await new Promise((r) => setTimeout(r, delay));
      }
    }
  }
  throw lastError;
}

function parseAuth(): { username: string; password: string } {
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

function basicAuthHeader(): string {
  const { username, password } = parseAuth();
  const encoded = Buffer.from(`${username}:${password}`).toString("base64");
  return `Basic ${encoded}`;
}

async function getOrCreateSession(
  credentialId: string,
  chatId: string,
): Promise<string> {
  const sessionKey = `${credentialId}:${chatId}`;
  const existing = sessions.get(sessionKey);
  if (existing) return existing;

  log(`Creating new OpenCode session for ${sessionKey}`);

  const resp = await fetch(`${backendConfig.base_url}/session`, {
    method: "POST",
    headers: {
      Authorization: basicAuthHeader(),
    },
    signal: AbortSignal.timeout(30_000),
  });

  if (!resp.ok) {
    const body = await resp.text();
    throw new Error(
      `Failed to create OpenCode session: ${resp.status} ${body}`,
    );
  }

  const data = (await resp.json()) as { id: string };
  const sessionId = data.id;
  // FIFO eviction: Map iterates in insertion order, so first key is earliest inserted
  if (sessions.size >= MAX_SESSIONS) {
    const firstInsertedKey = sessions.keys().next().value;
    if (firstInsertedKey !== undefined) sessions.delete(firstInsertedKey);
  }
  sessions.set(sessionKey, sessionId);
  log(`Session created: ${sessionId} for ${sessionKey}`);
  return sessionId;
}

async function sendToOpenCode(message: InboundMessage): Promise<string> {
  const chatId = message.source.chat_id;
  const sessionId = await getOrCreateSession(message.credential_id, chatId);

  log(
    `Sending message to OpenCode session=${sessionId} chat=${chatId}`,
  );

  const msgBody = {
    model: backendConfig.model,
    parts: [{ type: "text", text: message.text }],
  };

  const resp = await fetch(
    `${backendConfig.base_url}/session/${sessionId}/message`,
    {
      method: "POST",
      headers: {
        Authorization: basicAuthHeader(),
        "Content-Type": "application/json",
      },
      body: JSON.stringify(msgBody),
      signal: AbortSignal.timeout(60_000),
    },
  );

  if (!resp.ok) {
    const body = await resp.text();
    // Invalidate stale session on auth/not-found errors so next request recreates it
    if (resp.status === 401 || resp.status === 403 || resp.status === 404) {
      const sessionKey = `${message.credential_id}:${chatId}`;
      sessions.delete(sessionKey);
      log(`Invalidated stale session for ${sessionKey} (HTTP ${resp.status})`);
    }
    throw new Error(
      `OpenCode message failed: ${resp.status} ${body}`,
    );
  }

  const respBody = (await resp.json()) as {
    parts?: Array<{ type: string; text?: string }>;
  };

  const aiResponse =
    respBody.parts
      ?.filter((p) => p.type === "text" && p.text)
      .map((p) => p.text)
      .join("\n\n") ?? "";

  return aiResponse;
}

async function relayToGateway(
  credentialId: string,
  chatId: string,
  text: string,
): Promise<void> {
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
      signal: AbortSignal.timeout(15_000),
    });

    if (!resp.ok) {
      const body = await resp.text();
      throw new Error(`Gateway relay failed: ${resp.status} ${body}`);
    }
  });
}

const app = Fastify({ logger: false });

app.get("/health", async () => {
  return { status: "ok" };
});

app.post<{ Body: InboundMessage }>("/send", async (request, reply) => {
  const authHeader = request.headers.authorization;
  if (!authHeader || !verifyBearer(authHeader, BACKEND_TOKEN)) {
    reply.status(401);
    return { error: "Unauthorized" };
  }

  const message = request.body;
  const chatId = message.source.chat_id;
  const who =
    message.source.from.display_name ??
    message.source.from.username ??
    message.source.from.id;

  const truncatedText = message.text.length > 80 ? message.text.slice(0, 80) + "..." : message.text;
  log(`Received message from ${who} in chat ${chatId}: ${truncatedText}`);

  try {
    const aiResponse = await sendToOpenCode(message);

    if (!aiResponse || aiResponse.trim().length === 0) {
      log(`OpenCode returned empty response for chat ${chatId}, skipping relay`);
      return { status: "ok" };
    }

    log(`Relaying OpenCode response to gateway for chat ${chatId}`);
    await relayToGateway(message.credential_id, chatId, aiResponse);

    return { status: "ok" };
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    log(`Error processing message: ${msg}`);
    reply.status(500);
    return { error: msg };
  }
});

let shuttingDown = false;

async function shutdown(signal: string): Promise<void> {
  if (shuttingDown) return;
  shuttingDown = true;

  log(`Received ${signal}, shutting down...`);
  try {
    await app.close();
  } catch (err) {
    log(`Error during shutdown: ${err}`);
  }
  log("Backend adapter stopped");
  process.exit(0);
}

process.on("SIGTERM", () => shutdown("SIGTERM"));
process.on("SIGINT", () => shutdown("SIGINT"));

async function main(): Promise<void> {
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

  await app.listen({
    port: BACKEND_PORT,
    host: process.env.BACKEND_HOST ?? "0.0.0.0",
  });
  log(`HTTP server listening on port ${BACKEND_PORT}`);
}

main().catch((err) => {
  log(`Fatal error: ${err}`);
  process.exit(1);
});
