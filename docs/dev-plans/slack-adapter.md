# Slack Adapter — Development Plan

**Issue:** #10
**Status:** Ready
**Blocked by:** None (prerequisites done: message format #15, E2E framework #12, Telegram reference #13)
**Reference:** Telegram adapter at `adapters/telegram/`

---

## Overview

Bridges Slack workspaces to the gateway using the Bolt framework in Socket Mode, which means no public URL is required. One adapter instance = one Slack app installation, covering one workspace.

## Architecture

The adapter follows the same external subprocess model as the Telegram adapter. The gateway spawns it with env vars, the adapter connects to Slack via Socket Mode (a persistent WebSocket managed by `@slack/bolt`), and forwards inbound messages to `${GATEWAY_URL}/api/v1/adapter/inbound`. Outbound messages arrive via `POST /send` from the gateway.

Socket Mode is the key difference from Telegram: instead of polling or requiring a public webhook URL, Slack pushes events over a WebSocket connection that the adapter initiates. This makes it work in any network environment.

```
Slack Events API
       |
  Socket Mode WS (bolt manages)
       |
  adapter (Fastify)  <--POST /send--  msg-gateway
       |
  POST /api/v1/adapter/inbound  -->  msg-gateway
```

## Directory Structure

```
adapters/slack/
├── adapter.json          # Adapter manifest
├── package.json          # Node dependencies
├── tsconfig.json         # TypeScript config (identical to Telegram)
└── src/
    └── main.ts           # Full adapter implementation (~380 lines)
```

Mirror the Telegram adapter's flat structure exactly.

## Dependencies

```json
{
  "dependencies": {
    "fastify": "^5.0.0",
    "@slack/bolt": "^4.0.0"
  },
  "devDependencies": {
    "@types/node": "^20.0.0",
    "typescript": "^5.0.0"
  }
}
```

`@slack/bolt` v4 bundles `@slack/web-api` and `@slack/socket-mode` — no need to install them separately. Requires Node.js 18+; enforce `"node": ">=20"` to match the Telegram adapter.

## Credential Config

The `config` field in the credential entry (passed as `CREDENTIAL_CONFIG` JSON):

```json
{
  "app_token": "xapp-1-...",
  "channel_ids": ["C01234567", "C09876543"]
}
```

- `app_token`: **Required.** The app-level token (starts with `xapp-`) used for Socket Mode. This is separate from the bot token.
- `channel_ids`: Optional. If set, only process messages from these channels.

The bot token (`xoxb-...`) comes from `CREDENTIAL_TOKEN` as usual.

## Implementation

### 1. Scaffolding

**`adapter.json`**:
```json
{
  "name": "slack",
  "version": "1.0.0",
  "command": "node",
  "args": ["dist/main.js"]
}
```

**`package.json`**:
```json
{
  "name": "slack-adapter",
  "version": "1.0.0",
  "private": true,
  "engines": { "node": ">=20" },
  "scripts": {
    "build": "tsc",
    "start": "node dist/main.js"
  },
  "dependencies": {
    "fastify": "^5.0.0",
    "@slack/bolt": "^4.0.0"
  },
  "devDependencies": {
    "@types/node": "^20.0.0",
    "typescript": "^5.0.0"
  }
}
```

**`tsconfig.json`** — copy verbatim from `adapters/telegram/tsconfig.json`. No changes needed.

**`src/main.ts`** structure (follow Telegram's layout):
1. Env var parsing + config parsing
2. `log()` helper writing to stderr
3. `retry()` helper (copy from Telegram)
4. `InboundPayload` and `SendRequest` interfaces
5. Bolt `App` setup in Socket Mode
6. `forwardToGateway()` (copy from Telegram)
7. `app.message()` event handler
8. `sendOutbound()` function
9. Fastify server with `/health` and `/send`
10. `shutdown()` + signal handlers
11. `main()` entry point

### 2. Inbound (Slack → Gateway)

**Bolt app setup:**

```typescript
import { App, LogLevel } from "@slack/bolt";

const boltApp = new App({
  token: CREDENTIAL_TOKEN,           // xoxb-... bot token
  appToken: config.app_token,        // xapp-... app-level token
  socketMode: true,
  logLevel: LogLevel.ERROR,          // suppress bolt's own logging; we use our own
});
```

**`message` event handler:**

```typescript
boltApp.message(async ({ message, client }) => {
  // message is typed as GenericMessageEvent | BotMessageEvent | ...
  // Define a local SlackMessage interface with the fields we need:
  //   { channel: string; ts: string; text?: string; user?: string; thread_ts?: string;
  //     subtype?: string; bot_id?: string; files?: SlackFile[]; team?: string }
  const msg = message as SlackMessage;

  // Skip bot messages
  if (msg.subtype === "bot_message" || msg.bot_id) return;

  // Apply channel filter
  if (channelIds.length > 0 && !channelIds.includes(msg.channel)) return;

  // ... extract content and forward
});
```

Filter logic:
- Skip messages with `subtype === "bot_message"` or where `bot_id` is set
- Skip messages with `subtype === "message_deleted"` or `"message_changed"` (these are edit/delete events, not new messages)
- If `channel_ids` is configured, skip messages from other channels

Message content extraction:
- `msg.text` is the message text (may contain Slack mrkdwn formatting — pass as-is)
- `msg.files` is an optional array of file objects. Each file has: `.url_private` (requires auth header), `.name`, `.mimetype`, `.size`, `.id`
- If both `msg.text` and `msg.files` are empty/absent, skip the message

File handling: Slack file URLs (`url_private`) require an `Authorization: Bearer {bot_token}` header to download. Pass the URL directly in the `files` array — the gateway's file cache will handle the download. The gateway needs to know to add the auth header. Two options:
  1. The adapter downloads the file itself, saves to a temp path, and passes a local URL (simpler, avoids gateway needing Slack-specific auth logic).
  2. Pass the URL with auth embedded somehow.

**Recommended approach:** The adapter downloads the file using `fetch` with the auth header, saves it to a temp file under `/tmp/slack-{instance_id}/`, and passes a `file://` URL or a local HTTP URL served by the adapter itself. The cleanest solution is to serve the file from the adapter's Fastify server at `GET /files/{fileId}` and pass `http://localhost:{ADAPTER_PORT}/files/{fileId}` as the URL. The gateway then downloads it from the adapter.

`extra_data` to populate:
```typescript
extra_data: {
  thread_ts: msg.thread_ts ?? undefined,
  channel_name: await resolveChannelName(client, msg.channel),
  team_id: msg.team ?? undefined,
}
```

`chat_id`: Use `msg.channel` (the channel ID, e.g. `C01234567`).

`message_id`: Use `msg.ts` (the Slack timestamp string, e.g. `"1234567890.123456"`).

`reply_to_message_id`: If `msg.thread_ts` exists and differs from `msg.ts`, this is a threaded reply. Use `msg.thread_ts` as `reply_to_message_id`.

`from`:
```typescript
from: {
  id: msg.user,
  username: msg.user,           // Slack doesn't expose username in the event; use user ID
  display_name: await resolveDisplayName(client, msg.user),
}
```

`resolveDisplayName()` calls `client.users.info({ user: msg.user })` and returns `result.user?.real_name ?? result.user?.name`. Cache results in a `Map<string, string>` to avoid repeated API calls (user names don't change often).

`resolveChannelName()` calls `client.conversations.info({ channel })` and returns `result.channel?.name`. Cache similarly.

### 3. Outbound (Gateway → Slack)

**`POST /send` handler** receives a `SendRequest` body. The `sendOutbound()` function must:

1. Get the Slack Web API client from the bolt app: `boltApp.client`.

2. Text-only send:
   ```typescript
   const result = await boltApp.client.chat.postMessage({
     channel: body.chat_id,
     text: body.text ?? "",
     thread_ts: body.extra_data?.thread_ts as string | undefined,
   });
   return result.ts!;
   ```

3. File sends: Send text first via `chat.postMessage` to get a `ts`, then upload files into that message's thread:
   ```typescript
   // Step 1: Post the text message (returns ts)
   const textResult = await boltApp.client.chat.postMessage({
     channel: body.chat_id,
     text: body.text || "Attached files",
     thread_ts: body.extra_data?.thread_ts as string | undefined,
   });
   const ts = textResult.ts!;

   // Step 2: Upload files into the thread
   for (const filePath of body.file_paths ?? []) {
     const fileBuffer = await fs.promises.readFile(filePath);
     await boltApp.client.files.uploadV2({
       channel_id: body.chat_id,
       file: fileBuffer,
       filename: path.basename(filePath),
       thread_ts: ts,  // attach to the text message's thread
     });
   }
   return ts;
   ```
   This ensures we always have a `ts` to return as `protocol_message_id`. The `files.uploadV2` method returns `{ ok: true, files: [...] }` — not a `ts` — so we use `chat.postMessage` as the anchor.

4. Reply handling: If `reply_to_message_id` is set, pass it as `thread_ts` in `chat.postMessage`. This posts into the thread.

5. Return `{ protocol_message_id: ts }` where `ts` is the Slack message timestamp from `chat.postMessage`.

Rate limits: `chat.postMessage` is a Tier 3 method (~1 req/sec burst). Bolt's built-in retry handler manages 429 responses automatically. The `retry()` helper is still useful for network errors.

### 4. Platform-Specific Considerations

**Two tokens:** Slack requires both a bot token (`xoxb-`) for API calls and an app-level token (`xapp-`) for Socket Mode. The bot token goes in `CREDENTIAL_TOKEN`; the app token goes in `config.app_token`. If `app_token` is missing, the adapter must fail fast with a clear error message.

**Socket Mode setup:** In the Slack app settings, go to "Socket Mode" and enable it. Generate an app-level token with the `connections:write` scope. This is separate from the OAuth scopes for the bot token.

**Required OAuth scopes for the bot token:**
- `channels:history` — read messages in public channels
- `groups:history` — read messages in private channels
- `im:history` — read DMs
- `chat:write` — send messages
- `files:read` — access file metadata and download URLs
- `files:write` — upload files
- `users:read` — resolve user display names
- `channels:read` — resolve channel names

**Slack `ts` as message ID:** Slack uses a Unix timestamp with microseconds (e.g. `"1234567890.123456"`) as the message ID. It's a string, not a number. Preserve it exactly — it's also used as `thread_ts` for threading.

**mrkdwn formatting:** Slack uses its own markup (`*bold*`, `_italic_`, `<URL|text>`). Pass `msg.text` as-is to the gateway without converting. The backend agent can handle it or ignore it.

**E2E testing approach:** Socket Mode uses a WebSocket to Slack's servers, which can't be mocked easily. Use the same interface-injection pattern as Discord: export a `createAdapter(slackClient: SlackClientInterface)` factory from `main.ts`. Tests inject a mock client. The Fastify server still runs on a real port. For outbound tests, the mock client records `chat.postMessage` calls and lets tests assert on them.

## E2E Testing

### Mock Server

The mock server simulates the Slack Web API endpoints that the adapter calls:

- `POST /api/chat.postMessage` — records the call, returns `{"ok": true, "ts": "1234567890.123456"}`
- `POST /api/files.uploadV2` — records the call, returns `{"ok": true, "files": [{"id": "F123", "ts": "1234567890.123456"}]}`
- `GET /api/users.info` — returns a mock user object
- `GET /api/conversations.info` — returns a mock channel object

The mock server is a Fastify instance started on a random port. The adapter's bolt client is configured to point at this mock server's base URL instead of `https://slack.com`.

For inbound testing, the test calls a test-only endpoint on the adapter (`POST /test/trigger-inbound`) that fires the message event handler directly with a synthetic event payload. This avoids needing to mock the Socket Mode WebSocket.

### Scenarios

```gherkin
Feature: Slack adapter inbound

  Scenario: Text message forwarded to gateway
    Given the Slack adapter is running
    And the mock gateway is listening
    When a Slack message event fires with text "hello from slack"
    Then the adapter POSTs to /api/v1/adapter/inbound
    And the payload contains chat_id equal to the channel ID
    And the payload contains text "hello from slack"
    And the payload contains from.id equal to the user ID

  Scenario: Message with file forwarded to gateway
    Given the Slack adapter is running
    And the mock gateway is listening
    When a Slack message event fires with one file attachment
    Then the adapter POSTs to /api/v1/adapter/inbound
    And the payload files array contains one entry with a local URL

  Scenario: Bot message is ignored
    Given the Slack adapter is running
    When a Slack message event fires with subtype "bot_message"
    Then the adapter does NOT POST to /api/v1/adapter/inbound

  Scenario: Message filtered by channel_id
    Given the Slack adapter is running with channel_ids configured
    When a Slack message event fires from a non-configured channel
    Then the adapter does NOT POST to /api/v1/adapter/inbound

  Scenario: Threaded reply sets reply_to_message_id
    Given the Slack adapter is running
    When a Slack message event fires with thread_ts set
    Then the inbound payload contains reply_to_message_id equal to thread_ts
    And extra_data.thread_ts is set

Feature: Slack adapter outbound

  Scenario: Text message sent to channel
    Given the Slack adapter is running
    And the mock Slack API is listening
    When the gateway POSTs to /send with chat_id and text "hello slack"
    Then the adapter calls POST /api/chat.postMessage with channel and text
    And the response contains protocol_message_id

  Scenario: File sent to channel
    Given the Slack adapter is running
    And a temp file exists at a valid absolute path
    When the gateway POSTs to /send with file_paths containing that path
    Then the adapter calls POST /api/files.uploadV2
    And the response contains protocol_message_id

  Scenario: Reply posted into thread
    Given the Slack adapter is running
    When the gateway POSTs to /send with extra_data.thread_ts set
    Then the adapter calls chat.postMessage with thread_ts set
```

### Test Gateway Integration

In `tests/test-gateway.ts`, add a `startWithSlack()` method:

```typescript
async startWithSlack(options: {
  channelIds?: string[];
}): Promise<{ mockSlackApi: MockSlackApi; triggerInbound: (event: SlackMessageEvent) => Promise<void> }> {
  const mockSlackApi = new MockSlackApi();
  await mockSlackApi.start();

  await this.addCredential("slack_test", {
    adapter: "slack",
    token: "xoxb-test-token",
    active: true,
    config: {
      app_token: "xapp-test-token",
      channel_ids: options.channelIds,
      api_base_url: mockSlackApi.baseUrl,  // redirects bolt's Web API calls
    },
  });

  // triggerInbound calls the adapter's test endpoint to fire a synthetic event
  const triggerInbound = async (event: SlackMessageEvent) => {
    await fetch(`http://localhost:${adapterPort}/test/trigger-inbound`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(event),
    });
  };

  return { mockSlackApi, triggerInbound };
}
```

## Config Example

Add to `config.example.json` under `credentials`:

```json
"my_slack": {
  "adapter": "slack",
  "token": "${SLACK_BOT_TOKEN}",
  "active": true,
  "config": {
    "app_token": "${SLACK_APP_TOKEN}",
    "channel_ids": ["C01234567"]
  },
  "route": {
    "channel": "slack"
  }
}
```

`SLACK_BOT_TOKEN` starts with `xoxb-`. `SLACK_APP_TOKEN` starts with `xapp-`. Both are required.

## Checklist

- [ ] Create `adapters/slack/adapter.json`
- [ ] Create `adapters/slack/package.json` with @slack/bolt ^4 and fastify ^5
- [ ] Create `adapters/slack/tsconfig.json` (copy from Telegram)
- [ ] Create `adapters/slack/src/main.ts`
  - [ ] Parse env vars and `CREDENTIAL_CONFIG`
  - [ ] Validate `app_token` is present; fail fast with clear error if missing
  - [ ] Implement `log()` and `retry()` helpers
  - [ ] Define `InboundPayload` and `SendRequest` interfaces
  - [ ] Set up Bolt `App` in Socket Mode with `logLevel: LogLevel.ERROR`
  - [ ] Implement `forwardToGateway()` (copy from Telegram)
  - [ ] Implement `message` event handler with bot/subtype/channel filtering
  - [ ] Implement user display name resolution with in-memory cache
  - [ ] Implement channel name resolution with in-memory cache
  - [ ] Implement file download + local serving via Fastify `/files/{id}` route
  - [ ] Implement `sendOutbound()` with text, file (uploadV2), and thread reply support
  - [ ] Implement `validateFilePath()` (copy from Telegram)
  - [ ] Set up Fastify with `GET /health`, `POST /send`, `GET /files/:id`
  - [ ] Add `POST /test/trigger-inbound` endpoint (guarded by `NODE_ENV !== "production"`)
  - [ ] Implement `shutdown()` with SIGTERM/SIGINT handlers (stop bolt app + close Fastify)
  - [ ] Implement `main()` with `boltApp.start()` and startup logging
- [ ] Run `npm run build` — zero TypeScript errors
- [ ] Write E2E mock server (`MockSlackApi`) in `tests/`
- [ ] Write E2E scenarios: inbound text, inbound file, bot filter, channel filter, threaded reply, outbound text, outbound file, outbound thread reply
- [ ] Add `startWithSlack()` to `tests/test-gateway.ts`
- [ ] Add credential example to `config.example.json`
- [ ] Document required OAuth scopes in a comment block in `main.ts`
- [ ] Document Socket Mode setup requirement (app-level token, `connections:write` scope) in a comment
