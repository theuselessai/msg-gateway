import Fastify from "fastify";
import { Bot, InputFile, type Context } from "grammy";
import * as fs from "fs";
import * as path from "path";

const INSTANCE_ID = process.env.INSTANCE_ID ?? "unknown";
const ADAPTER_PORT = parseInt(process.env.ADAPTER_PORT ?? "9001", 10);
const GATEWAY_URL = process.env.GATEWAY_URL ?? "http://localhost:8080";
const CREDENTIAL_ID = process.env.CREDENTIAL_ID ?? "unknown";
const CREDENTIAL_TOKEN = process.env.CREDENTIAL_TOKEN ?? "";

function log(msg: string): void {
  const ts = new Date().toISOString();
  process.stderr.write(`[${ts}] [${INSTANCE_ID}] ${msg}\n`);
}

const bot = new Bot(CREDENTIAL_TOKEN);

const IMAGE_EXTENSIONS = new Set([".jpg", ".jpeg", ".png", ".gif", ".webp"]);

async function retry<T>(
  fn: () => Promise<T>,
  retries = 3,
  baseDelay = 1000
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

interface FileAttachment {
  url: string;
  filename: string;
  mime_type: string;
  auth_header?: string;
}

interface InboundPayload {
  instance_id: string;
  chat_id: string;
  message_id: string;
  reply_to_message_id?: string;
  text: string;
  from: {
    id: string;
    username?: string;
    display_name?: string;
  };
  timestamp: string;
  files: FileAttachment[];
  extra_data: Record<string, unknown>;
}

async function forwardToGateway(payload: InboundPayload): Promise<void> {
  const url = `${GATEWAY_URL}/api/v1/adapter/inbound`;
  await retry(async () => {
    const resp = await fetch(url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    });
    if (!resp.ok) {
      const body = await resp.text();
      throw new Error(`Gateway returned ${resp.status}: ${body}`);
    }
  });
}

async function resolveFileUrl(fileId: string): Promise<string | null> {
  try {
    const file = await bot.api.getFile(fileId);
    if (file.file_path) {
      return `https://api.telegram.org/file/bot${CREDENTIAL_TOKEN}/${file.file_path}`;
    }
  } catch (err) {
    log(`Failed to resolve file ${fileId}: ${err}`);
  }
  return null;
}

bot.on("message", async (ctx: Context) => {
  const message = ctx.message;
  if (!message) return;

  let text = "";
  const files: FileAttachment[] = [];

  if (message.photo && message.photo.length > 0) {
    const largest = message.photo.reduce((a, b) =>
      (a.file_size ?? 0) >= (b.file_size ?? 0) ? a : b
    );
    const url = await resolveFileUrl(largest.file_id);
    if (url) {
      files.push({
        url,
        filename: `photo_${message.message_id}.jpg`,
        mime_type: "image/jpeg",
        auth_header: `Bot ${CREDENTIAL_TOKEN}`,
      });
    }
    text = message.caption ?? "[Photo]";
  } else if (message.document) {
    const doc = message.document;
    const url = await resolveFileUrl(doc.file_id);
    if (url) {
      files.push({
        url,
        filename: doc.file_name ?? `file_${message.message_id}`,
        mime_type: doc.mime_type ?? "application/octet-stream",
        auth_header: `Bot ${CREDENTIAL_TOKEN}`,
      });
    }
    text = message.caption ?? `[Document: ${doc.file_name ?? "file"}]`;
  } else if (message.voice) {
    const voice = message.voice;
    const url = await resolveFileUrl(voice.file_id);
    if (url) {
      files.push({
        url,
        filename: `voice_${message.message_id}.ogg`,
        mime_type: voice.mime_type ?? "audio/ogg",
        auth_header: `Bot ${CREDENTIAL_TOKEN}`,
      });
    }
    text = "[Voice message]";
  } else if (message.text) {
    text = message.text;
  }

  if (!text && files.length === 0) {
    log(`Skipping empty message ${message.message_id}`);
    return;
  }

  const fromUser = message.from;
  const firstName = fromUser?.first_name ?? "";
  const lastName = fromUser?.last_name ?? "";
  const displayName = `${firstName} ${lastName}`.trim() || undefined;

  const payload: InboundPayload = {
    instance_id: INSTANCE_ID,
    chat_id: String(message.chat.id),
    message_id: String(message.message_id),
    ...(message.reply_to_message
      ? {
          reply_to_message_id: String(
            message.reply_to_message.message_id
          ),
        }
      : {}),
    text,
    from: {
      id: String(fromUser?.id ?? "unknown"),
      username: fromUser?.username,
      display_name: displayName,
    },
    timestamp: new Date().toISOString(),
    files,
    extra_data: {},
  };

  const who = displayName ?? fromUser?.username ?? "unknown";
  if (files.length > 0) {
    log(
      `Received file from ${who}: ${files.map((f) => f.filename).join(", ")}`
    );
  } else {
    log(`Received message from ${who}: ${text.slice(0, 50)}...`);
  }

  try {
    await forwardToGateway(payload);
    log(`Message ${message.message_id} forwarded to gateway`);
  } catch (err) {
    log(`Failed to forward message ${message.message_id}: ${err}`);
  }
});

interface SendRequest {
  chat_id: string;
  text?: string;
  reply_to_message_id?: string;
  file_paths?: string[];
  extra_data?: Record<string, unknown>;
}

async function sendOutbound(body: SendRequest): Promise<string> {
  const chatId = body.chat_id;
  const text = body.text ?? "";
  const replyTo = body.reply_to_message_id;
  const filePaths = body.file_paths ?? [];

  const replyParams = replyTo
    ? { message_id: parseInt(replyTo, 10) }
    : undefined;

  let lastMessageId = "";

  if (filePaths.length === 0) {
    log(`Sending message to chat ${chatId}: ${text.slice(0, 50)}...`);
    const sent = await bot.api.sendMessage(chatId, text, {
      reply_parameters: replyParams,
    });
    lastMessageId = String(sent.message_id);
  } else {
    for (let i = 0; i < filePaths.length; i++) {
      const filePath = filePaths[i];
      const ext = path.extname(filePath).toLowerCase();
      const isImage = IMAGE_EXTENSIONS.has(ext);
      const caption = i === 0 && text ? text : undefined;
      const reply = i === 0 ? replyParams : undefined;

      const fileBuffer = fs.readFileSync(filePath);
      const filename = path.basename(filePath);
      const inputFile = new InputFile(fileBuffer, filename);

      if (isImage) {
        log(`Sending photo to chat ${chatId}: ${filePath}`);
        const sent = await bot.api.sendPhoto(chatId, inputFile, {
          caption,
          reply_parameters: reply,
        });
        lastMessageId = String(sent.message_id);
      } else {
        log(`Sending document to chat ${chatId}: ${filePath}`);
        const sent = await bot.api.sendDocument(chatId, inputFile, {
          caption,
          reply_parameters: reply,
        });
        lastMessageId = String(sent.message_id);
      }
    }
  }

  return lastMessageId;
}

const app = Fastify({ logger: false });

app.get("/health", async () => {
  return { status: "ok" };
});

app.post<{ Body: SendRequest }>("/send", async (request, reply) => {
  try {
    const messageId = await sendOutbound(request.body);
    return { protocol_message_id: messageId };
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    log(`Send error: ${msg}`);
    reply.status(500);
    return { error: msg };
  }
});

let shuttingDown = false;

async function shutdown(signal: string): Promise<void> {
  if (shuttingDown) return;
  shuttingDown = true;

  log(`Received ${signal}, shutting down...`);

  try { bot.stop(); } catch { /* noop */ }
  try { await app.close(); } catch { /* noop */ }

  log("Adapter stopped");
  process.exit(0);
}

process.on("SIGTERM", () => shutdown("SIGTERM"));
process.on("SIGINT", () => shutdown("SIGINT"));

async function main(): Promise<void> {
  log("Starting Telegram adapter");
  log(`  Port: ${ADAPTER_PORT}`);
  log(`  Gateway: ${GATEWAY_URL}`);
  log(`  Credential: ${CREDENTIAL_ID}`);

  try {
    const me = await bot.api.getMe();
    log(`  Bot: @${me.username} (${me.first_name})`);
  } catch (err) {
    log(`WARNING: Could not verify bot token: ${err}`);
  }

  await app.listen({ port: ADAPTER_PORT, host: "127.0.0.1" });
  log(`HTTP server listening on port ${ADAPTER_PORT}`);

  bot.start({
    allowed_updates: ["message"],
    onStart: () => log("Telegram polling started"),
  });
}

main().catch((err) => {
  log(`Fatal error: ${err}`);
  process.exit(1);
});
