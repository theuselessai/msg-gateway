# Email Adapter — Development Plan

**Issue:** #11
**Status:** Ready
**Blocked by:** None (prerequisites done: message format #15, E2E framework #12, Telegram reference #13)
**Reference:** Telegram adapter at `adapters/telegram/`

---

## Overview

Bridges email to the gateway using IMAP IDLE for inbound (push notifications for new messages) and SMTP for outbound. One adapter instance = one email account, covering one inbox.

## Architecture

The adapter follows the same external subprocess model as the Telegram adapter. The gateway spawns it with env vars, the adapter connects to an IMAP server and maintains an IDLE connection for real-time delivery, and forwards inbound emails to `${GATEWAY_URL}/api/v1/adapter/inbound`. Outbound emails arrive via `POST /send` from the gateway and are sent via SMTP.

Email is the most structurally different adapter: there's no SDK that wraps both inbound and outbound, authentication uses multiple credential sets (IMAP + SMTP), and message threading relies on email headers rather than platform IDs.

```
IMAP Server (IDLE)
       |
  imapflow connection
       |
  adapter (Fastify)  <--POST /send--  msg-gateway
       |
  POST /api/v1/adapter/inbound  -->  msg-gateway
       |
  SMTP Server  <--  nodemailer
```

## Directory Structure

```
adapters/email/
├── adapter.json          # Adapter manifest
├── package.json          # Node dependencies
├── tsconfig.json         # TypeScript config (identical to Telegram)
└── src/
    └── main.ts           # Full adapter implementation (~450 lines)
```

Mirror the Telegram adapter's flat structure. Email's implementation is longer due to MIME parsing and IMAP state management.

## Dependencies

```json
{
  "dependencies": {
    "fastify": "^5.0.0",
    "imapflow": "^1.0.0",
    "nodemailer": "^6.0.0",
    "mailparser": "^3.0.0"
  },
  "devDependencies": {
    "@types/node": "^20.0.0",
    "@types/nodemailer": "^6.0.0",
    "typescript": "^5.0.0"
  }
}
```

- `imapflow`: Modern IMAP client with IDLE support and async/await API. No callback hell.
- `nodemailer`: Standard SMTP client. Well-typed via `@types/nodemailer`.
- `mailparser`: Parses raw MIME email into structured objects (headers, text, html, attachments). Handles encoding, multipart, etc.

Requires Node.js 18+; enforce `"node": ">=20"` to match the Telegram adapter.

## Credential Config

Unlike other adapters, email needs two sets of credentials (IMAP + SMTP). Both live in the `config` field. `CREDENTIAL_TOKEN` is not used for auth — set it to a dummy value like `"email"` in the credential entry.

```json
{
  "imap": {
    "host": "imap.example.com",
    "port": 993,
    "auth": {
      "user": "bot@example.com",
      "pass": "secret"
    },
    "tls": true
  },
  "smtp": {
    "host": "smtp.example.com",
    "port": 587,
    "auth": {
      "user": "bot@example.com",
      "pass": "secret"
    },
    "tls": false
  },
  "default_from": "Bot Name <bot@example.com>",
  "poll_interval_seconds": 60
}
```

- `imap.tls`: If `true`, use TLS on connect (port 993). If `false`, use STARTTLS (port 143).
- `smtp.tls`: If `false`, nodemailer uses STARTTLS on port 587. If `true`, uses TLS on port 465.
- `default_from`: The `From` header for outbound emails. Required.
- `poll_interval_seconds`: Fallback polling interval if the IMAP server doesn't support IDLE. Defaults to 60.

## Implementation

### 1. Scaffolding

**`adapter.json`**:
```json
{
  "name": "email",
  "version": "1.0.0",
  "command": "node",
  "args": ["dist/main.js"]
}
```

**`package.json`**:
```json
{
  "name": "email-adapter",
  "version": "1.0.0",
  "private": true,
  "engines": { "node": ">=20" },
  "scripts": {
    "build": "tsc",
    "start": "node dist/main.js"
  },
  "dependencies": {
    "fastify": "^5.0.0",
    "imapflow": "^1.0.0",
    "nodemailer": "^6.0.0",
    "mailparser": "^3.0.0"
  },
  "devDependencies": {
    "@types/node": "^20.0.0",
    "@types/nodemailer": "^6.0.0",
    "typescript": "^5.0.0"
  }
}
```

**`tsconfig.json`** — copy verbatim from `adapters/telegram/tsconfig.json`. No changes needed.

**`src/main.ts`** structure (follow Telegram's layout):
1. Env var parsing + config parsing + config validation
2. `log()` helper writing to stderr
3. `retry()` helper (copy from Telegram)
4. `InboundPayload` and `SendRequest` interfaces
5. imapflow `ImapFlow` client setup
6. `forwardToGateway()` (copy from Telegram)
7. `processEmail()` — parses a raw email and calls `forwardToGateway()`
8. `startImapIdle()` — connects, selects INBOX, enters IDLE loop
9. `sendOutbound()` — nodemailer send
10. Fastify server with `/health` and `/send`
11. `shutdown()` + signal handlers
12. `main()` entry point

### 2. Inbound (Email → Gateway)

**IMAP connection setup:**

```typescript
import { ImapFlow } from "imapflow";

const client = new ImapFlow({
  host: config.imap.host,
  port: config.imap.port,
  secure: config.imap.tls ?? true,
  auth: {
    user: config.imap.auth.user,
    pass: config.imap.auth.pass,
  },
  logger: false,  // suppress imapflow's own logging
});
```

**IDLE loop in `startImapIdle()`:**

```typescript
await client.connect();
await client.mailboxOpen("INBOX");

// Process any messages that arrived while we were offline
await processNewMessages(client, lastSeenUid);

// Enter IDLE — imapflow emits "exists" when new messages arrive
client.on("exists", async () => {
  await processNewMessages(client, lastSeenUid);
});

await client.idle();  // blocks until connection drops or shutdown
```

`processNewMessages()` fetches messages with UID greater than `lastSeenUid`, processes each one, and updates `lastSeenUid`. Use `client.fetch("1:*", { uid: true, envelope: true, source: true })` to get raw message source.

**Reconnection:** Wrap `startImapIdle()` in a retry loop in `main()`. If the IMAP connection drops, wait `Math.min(30000, baseDelay * 2^attempt)` milliseconds and reconnect. Reset the attempt counter on successful connection.

**Fallback polling:** If the server doesn't support IDLE (imapflow will throw or the `idle()` call returns immediately), fall back to polling with `setInterval(processNewMessages, config.poll_interval_seconds * 1000)`.

**`processEmail()` — parsing a raw email:**

```typescript
import { simpleParser, ParsedMail } from "mailparser";

async function processEmail(rawSource: Buffer, uid: number): Promise<void> {
  const parsed: ParsedMail = await simpleParser(rawSource);

  // Extract text content
  const text = parsed.text ?? parsed.html ?? "";  // prefer plaintext

  // Extract sender
  const fromAddress = parsed.from?.value[0];
  const chatId = fromAddress?.address ?? "unknown@unknown";
  const displayName = fromAddress?.name || undefined;

  // Message ID from headers
  const messageId = parsed.messageId ?? `uid-${uid}`;

  // Threading headers
  const inReplyTo = parsed.inReplyTo ?? undefined;
  const references = Array.isArray(parsed.references)
    ? parsed.references.join(" ")
    : parsed.references ?? undefined;

  // File attachments
  const files: FileAttachment[] = [];
  for (const attachment of parsed.attachments ?? []) {
    if (!attachment.content) continue;
    const tempPath = await saveTempFile(attachment);
    files.push({
      url: `http://localhost:${ADAPTER_PORT}/files/${path.basename(tempPath)}`,
      filename: attachment.filename ?? "attachment",
      mime_type: attachment.contentType ?? "application/octet-stream",
    });
  }

  // Determine if this email was CC'd or BCC'd to us
  const ourAddress = config.imap.auth.user.toLowerCase();
  const toAddresses = (parsed.to?.value ?? []).map(a => a.address?.toLowerCase());
  const ccAddresses = (parsed.cc?.value ?? []).map(a => a.address?.toLowerCase());
  const isCc = !toAddresses.includes(ourAddress) && ccAddresses.includes(ourAddress);
  const isBcc = !toAddresses.includes(ourAddress) && !ccAddresses.includes(ourAddress);

  const payload: InboundPayload = {
    instance_id: INSTANCE_ID,
    chat_id: chatId,
    message_id: messageId,
    reply_to_message_id: inReplyTo,
    text: text.trim(),
    from: {
      id: chatId,
      username: chatId,
      display_name: displayName,
    },
    timestamp: (parsed.date ?? new Date()).toISOString(),
    files,
    extra_data: {
      subject: parsed.subject ?? "",
      to: toAddresses.filter(Boolean),
      cc: ccAddresses.filter(Boolean),
      in_reply_to: inReplyTo,
      references,
      html_body: parsed.html || undefined,
      is_cc: isCc,
      is_bcc: isBcc,
    },
  };

  await forwardToGateway(payload);
}
```

**Temp file serving:** Save attachment buffers to `/tmp/email-{INSTANCE_ID}/` with a UUID filename. Serve them from Fastify at `GET /files/:filename`. Clean up files after the gateway has had time to download them (e.g. delete after 5 minutes using `setTimeout`).

**`saveTempFile()` helper:**
```typescript
import * as crypto from "crypto";
import * as os from "os";

async function saveTempFile(attachment: Attachment): Promise<string> {
  const dir = path.join(os.tmpdir(), `email-${INSTANCE_ID}`);
  await fs.promises.mkdir(dir, { recursive: true });
  const ext = path.extname(attachment.filename ?? "") || "";
  const filename = `${crypto.randomUUID()}${ext}`;
  const filePath = path.join(dir, filename);
  await fs.promises.writeFile(filePath, attachment.content);
  // Schedule cleanup after 5 minutes
  setTimeout(() => fs.promises.unlink(filePath).catch(() => {}), 5 * 60 * 1000);
  return filePath;
}
```

### 3. Outbound (Gateway → Email)

**`POST /send` handler** receives a `SendRequest` body. The `sendOutbound()` function must:

1. Determine the recipient: `body.chat_id` is the email address to send to (the sender's address from the inbound message).

2. Build the nodemailer transport:
   ```typescript
   import nodemailer from "nodemailer";

   const transporter = nodemailer.createTransport({
     host: config.smtp.host,
     port: config.smtp.port,
     secure: config.smtp.tls ?? false,
     auth: {
       user: config.smtp.auth.user,
       pass: config.smtp.auth.pass,
     },
   });
   ```
   Create the transporter once at startup and reuse it.

3. Build the mail options:
   ```typescript
   const mailOptions: nodemailer.SendMailOptions = {
     from: config.default_from,
     to: body.chat_id,
     subject: body.extra_data?.subject as string | undefined ?? "Re: (no subject)",
     text: body.text ?? "",
   };
   ```

4. Threading: If `body.reply_to_message_id` is set, add `In-Reply-To` and `References` headers:
   ```typescript
   mailOptions.inReplyTo = body.reply_to_message_id;
   mailOptions.references = [
     body.extra_data?.references as string | undefined,
     body.reply_to_message_id,
   ].filter(Boolean).join(" ");
   ```

5. File attachments: For each path in `body.file_paths`, validate with `validateFilePath()` (copy from Telegram), read the buffer, and add to `mailOptions.attachments`:
   ```typescript
   mailOptions.attachments = filePaths.map(fp => ({
     filename: path.basename(fp),
     path: fp,
   }));
   ```

6. Send and return the message ID:
   ```typescript
   const info = await transporter.sendMail(mailOptions);
   return info.messageId;  // e.g. "<uuid@hostname>"
   ```

7. Return `{ protocol_message_id: messageId }`.

### 4. Platform-Specific Considerations

**CREDENTIAL_TOKEN is unused:** Set it to a placeholder like `"email"` in the credential config. Real auth is in `config.imap.auth` and `config.smtp.auth`. The adapter should log a warning if `CREDENTIAL_TOKEN` is empty but not fail.

**Email threading:** Proper threading requires preserving `Message-ID`, `In-Reply-To`, and `References` headers. The inbound handler stores these in `extra_data`. The outbound handler reads them back to set the correct headers. Without this, replies appear as new threads in email clients.

**HTML vs plaintext:** `mailparser` extracts both `parsed.text` and `parsed.html`. Always prefer `parsed.text` for the gateway's `text` field — it's cleaner for LLM processing. Pass `parsed.html` as `extra_data.html_body` so the backend can use it if needed. For outbound, send plaintext only unless the backend explicitly provides HTML (not in scope for v1).

**Large attachments:** Email attachments can be large. The temp file approach avoids holding large buffers in memory. The gateway's file cache handles size limits on its end.

**IMAP UID tracking:** Track the last processed UID in memory. On reconnect, fetch only messages with UID > lastSeenUid to avoid reprocessing. On first startup with no stored UID, process only messages received in the last 24 hours (use `SINCE` search criterion) to avoid flooding the gateway with old emails.

**IMAP IDLE timeout:** IMAP servers typically drop IDLE connections after 29 minutes. imapflow handles this by re-entering IDLE automatically. No manual keepalive needed.

**Gmail-specific:** Gmail requires enabling "Less secure app access" or using OAuth2. For simplicity, document that app passwords should be used with Gmail (Settings > Security > App passwords). OAuth2 support is out of scope for v1.

**E2E testing approach:** Mocking IMAP is complex. Use the `smtp-server` npm package to create a mock SMTP server for outbound testing. For inbound, abstract the IMAP client behind an `ImapAdapter` interface and inject a mock in tests. The mock fires the `processEmail()` function directly with a synthetic raw email buffer.

## E2E Testing

### Mock Server

**Outbound (SMTP):** Use the `smtp-server` npm package to create a mock SMTP server:

```typescript
import { SMTPServer } from "smtp-server";

const mockSmtp = new SMTPServer({
  authOptional: true,
  onData(stream, session, callback) {
    // collect the raw email
    let data = "";
    stream.on("data", chunk => data += chunk);
    stream.on("end", () => {
      receivedEmails.push(data);
      callback();
    });
  },
});
mockSmtp.listen(0);  // random port
```

The test configures the adapter's SMTP to point at this mock server.

**Inbound (IMAP):** Don't mock IMAP. Instead, export a `triggerInbound(rawEmail: Buffer)` function from `main.ts` that calls `processEmail()` directly. Tests call this via a test-only Fastify endpoint (`POST /test/trigger-inbound`) with a raw email body.

The test-only endpoint is only registered when `NODE_ENV !== "production"`.

### Scenarios

```gherkin
Feature: Email adapter inbound

  Scenario: Plain text email forwarded to gateway
    Given the Email adapter is running
    And the mock gateway is listening
    When a raw email arrives with text body "hello from email"
    Then the adapter POSTs to /api/v1/adapter/inbound
    And the payload contains chat_id equal to the sender's email address
    And the payload contains text "hello from email"
    And the payload contains from.id equal to the sender's email address

  Scenario: HTML email extracts plaintext for text field
    Given the Email adapter is running
    When a raw email arrives with HTML body and no plaintext alternative
    Then the inbound payload text contains the HTML content
    And extra_data.html_body contains the original HTML

  Scenario: Email with attachment forwarded to gateway
    Given the Email adapter is running
    And the mock gateway is listening
    When a raw email arrives with a PDF attachment
    Then the adapter POSTs to /api/v1/adapter/inbound
    And the payload files array contains one entry with a local URL
    And the file is accessible at that URL

  Scenario: Reply email sets reply_to_message_id
    Given the Email adapter is running
    When a raw email arrives with In-Reply-To header set
    Then the inbound payload contains reply_to_message_id equal to that header value
    And extra_data.in_reply_to is set
    And extra_data.references is set

  Scenario: CC'd email sets is_cc flag
    Given the Email adapter is running
    When a raw email arrives where our address is in CC not To
    Then extra_data.is_cc is true

Feature: Email adapter outbound

  Scenario: Text email sent via SMTP
    Given the Email adapter is running
    And the mock SMTP server is listening
    When the gateway POSTs to /send with chat_id "recipient@example.com" and text "hello email"
    Then the adapter sends an email to recipient@example.com
    And the email body contains "hello email"
    And the response contains protocol_message_id

  Scenario: Reply email preserves threading headers
    Given the Email adapter is running
    When the gateway POSTs to /send with reply_to_message_id and extra_data.references set
    Then the sent email has In-Reply-To header matching reply_to_message_id
    And the sent email has References header containing both the original and new IDs

  Scenario: Email with file attachment sent via SMTP
    Given the Email adapter is running
    And a temp file exists at a valid absolute path
    When the gateway POSTs to /send with file_paths containing that path
    Then the sent email has one attachment with the correct filename
```

### Test Gateway Integration

In `tests/test-gateway.ts`, add a `startWithEmail()` method:

```typescript
async startWithEmail(): Promise<{
  mockSmtp: MockSmtpServer;
  triggerInbound: (rawEmail: Buffer) => Promise<void>;
}> {
  const mockSmtp = new MockSmtpServer();
  await mockSmtp.start();

  await this.addCredential("email_test", {
    adapter: "email",
    token: "email",  // dummy value
    active: true,
    config: {
      imap: {
        host: "localhost",
        port: 10143,
        auth: { user: "test@example.com", pass: "test" },
        tls: false,
      },
      smtp: {
        host: "localhost",
        port: mockSmtp.port,
        auth: { user: "test@example.com", pass: "test" },
        tls: false,
      },
      default_from: "Test Bot <test@example.com>",
    },
  });

  const triggerInbound = async (rawEmail: Buffer) => {
    await fetch(`http://localhost:${adapterPort}/test/trigger-inbound`, {
      method: "POST",
      headers: { "Content-Type": "message/rfc822" },
      body: rawEmail,
    });
  };

  return { mockSmtp, triggerInbound };
}
```

`MockSmtpServer` wraps `smtp-server` and exposes `receivedEmails: ParsedMail[]` for assertions.

## Config Example

Add to `config.example.json` under `credentials`:

```json
"my_email": {
  "adapter": "email",
  "token": "email",
  "active": true,
  "config": {
    "imap": {
      "host": "imap.example.com",
      "port": 993,
      "auth": {
        "user": "${EMAIL_USER}",
        "pass": "${EMAIL_IMAP_PASS}"
      },
      "tls": true
    },
    "smtp": {
      "host": "smtp.example.com",
      "port": 587,
      "auth": {
        "user": "${EMAIL_USER}",
        "pass": "${EMAIL_SMTP_PASS}"
      },
      "tls": false
    },
    "default_from": "My Bot <${EMAIL_USER}>",
    "poll_interval_seconds": 60
  },
  "route": {
    "channel": "email"
  }
}
```

Note: `token` is set to the literal string `"email"` since IMAP/SMTP credentials live in `config`. The `${ENV_VAR}` syntax is supported by the gateway's config parser.

## Checklist

- [ ] Create `adapters/email/adapter.json`
- [ ] Create `adapters/email/package.json` with imapflow ^1, nodemailer ^6, mailparser ^3, fastify ^5
- [ ] Create `adapters/email/tsconfig.json` (copy from Telegram)
- [ ] Create `adapters/email/src/main.ts`
  - [ ] Parse env vars and `CREDENTIAL_CONFIG`
  - [ ] Validate required config fields (imap.host, imap.auth, smtp.host, smtp.auth, default_from); fail fast with clear errors
  - [ ] Implement `log()` and `retry()` helpers
  - [ ] Define `InboundPayload` and `SendRequest` interfaces
  - [ ] Set up imapflow `ImapFlow` client
  - [ ] Implement `forwardToGateway()` (copy from Telegram)
  - [ ] Implement `saveTempFile()` with UUID filenames and 5-minute cleanup
  - [ ] Implement `processEmail()` with mailparser, text/HTML extraction, attachment handling, threading header extraction, CC/BCC detection
  - [ ] Implement `processNewMessages()` with UID tracking and SINCE filter on first run
  - [ ] Implement `startImapIdle()` with reconnection loop and polling fallback
  - [ ] Create nodemailer transporter at startup
  - [ ] Implement `sendOutbound()` with text, attachments, threading headers (In-Reply-To, References)
  - [ ] Implement `validateFilePath()` (copy from Telegram)
  - [ ] Set up Fastify with `GET /health`, `POST /send`, `GET /files/:filename`
  - [ ] Add `POST /test/trigger-inbound` endpoint (guarded by `NODE_ENV !== "production"`)
  - [ ] Implement `shutdown()` with SIGTERM/SIGINT handlers (close IMAP client + transporter + Fastify)
  - [ ] Implement `main()` with startup validation and IMAP connection
- [ ] Run `npm run build` — zero TypeScript errors
- [ ] Add `smtp-server` to devDependencies for E2E tests
- [ ] Write `MockSmtpServer` wrapper in `tests/`
- [ ] Write E2E scenarios: inbound text, inbound HTML, inbound attachment, reply threading, CC detection, outbound text, outbound reply headers, outbound attachment
- [ ] Add `startWithEmail()` to `tests/test-gateway.ts`
- [ ] Add credential example to `config.example.json`
- [ ] Document Gmail app password requirement in a comment in `main.ts`
- [ ] Document that `CREDENTIAL_TOKEN` is unused (set to `"email"`) in a comment
