# Discord Adapter — Development Plan

**Issue:** #9
**Status:** Ready
**Blocked by:** None (prerequisites done: message format #15, E2E framework #12, Telegram reference #13)
**Reference:** Telegram adapter at `adapters/telegram/`

---

## Overview

Bridges Discord servers to the gateway by running a discord.js bot that listens for messages via the Discord Gateway WebSocket and exposes a Fastify HTTP server for outbound sends. One adapter instance = one bot token, potentially covering multiple guilds.

## Architecture

The adapter follows the same external subprocess model as the Telegram adapter. The gateway spawns it with env vars, the adapter connects to Discord using discord.js, and forwards inbound messages to `${GATEWAY_URL}/api/v1/adapter/inbound`. Outbound messages arrive via `POST /send` from the gateway.

Discord's real-time message delivery comes through discord.js's managed WebSocket connection to the Discord Gateway. The adapter does not need to expose a public URL for inbound — discord.js handles the connection internally, just like grammy does for Telegram long-polling.

```
Discord Gateway WS
       |
  discord.js Client
       |
  adapter (Fastify)  <--POST /send--  msg-gateway
       |
  POST /api/v1/adapter/inbound  -->  msg-gateway
```

## Directory Structure

```
adapters/discord/
├── adapter.json          # Adapter manifest
├── package.json          # Node dependencies
├── tsconfig.json         # TypeScript config (identical to Telegram)
└── src/
    └── main.ts           # Full adapter implementation (~350 lines)
```

No subdirectories needed. Mirror the Telegram adapter's flat structure exactly.

## Dependencies

```json
{
  "dependencies": {
    "fastify": "^5.0.0",
    "discord.js": "^14.0.0"
  },
  "devDependencies": {
    "@types/node": "^20.0.0",
    "typescript": "^5.0.0"
  }
}
```

discord.js v14 requires Node.js 18+. The adapter should enforce `"node": ">=20"` in `engines` to match the Telegram adapter.

## Credential Config

The `config` field in the credential entry (passed as `CREDENTIAL_CONFIG` JSON):

```json
{
  "guild_ids": ["123456789012345678"],
  "channel_ids": ["987654321098765432"],
  "api_root": "http://localhost:19001"
}
```

- `guild_ids`: Optional. If set, only process messages from these guilds (snowflake strings).
- `channel_ids`: Optional. If set, only process messages from these channels (snowflake strings).
- `api_root`: Optional. Overrides the Discord REST API base URL. Used in E2E tests to redirect REST calls to a mock server.

All three fields are optional. An empty config `{}` means the bot processes all messages it can see.

## Implementation

### 1. Scaffolding

**`adapter.json`** — identical pattern to Telegram:
```json
{
  "name": "discord",
  "version": "1.0.0",
  "command": "node",
  "args": ["dist/main.js"]
}
```

**`package.json`**:
```json
{
  "name": "discord-adapter",
  "version": "1.0.0",
  "private": true,
  "engines": { "node": ">=20" },
  "scripts": {
    "build": "tsc",
    "start": "node dist/main.js"
  },
  "dependencies": {
    "fastify": "^5.0.0",
    "discord.js": "^14.0.0"
  },
  "devDependencies": {
    "@types/node": "^20.0.0",
    "typescript": "^5.0.0"
  }
}
```

**`tsconfig.json`** — copy verbatim from `adapters/telegram/tsconfig.json`. No changes needed.

**`src/main.ts`** structure (follow Telegram's layout exactly):
1. Env var parsing + config parsing
2. `log()` helper writing to stderr
3. `retry()` helper (copy from Telegram)
4. `InboundPayload` and `SendRequest` interfaces
5. discord.js `Client` setup
6. `forwardToGateway()` (copy from Telegram, same signature)
7. `client.on("messageCreate", ...)` handler
8. `sendOutbound()` function
9. Fastify server with `/health` and `/send`
10. `shutdown()` + signal handlers
11. `main()` entry point

### 2. Inbound (Discord → Gateway)

**Client setup:**

```typescript
import { Client, GatewayIntentBits, Partials } from "discord.js";

const client = new Client({
  intents: [
    GatewayIntentBits.Guilds,
    GatewayIntentBits.GuildMessages,
    GatewayIntentBits.MessageContent,  // Privileged intent — must be enabled in Dev Portal
    GatewayIntentBits.DirectMessages,
  ],
  partials: [Partials.Channel],  // Required for DM support
  rest: apiRoot ? { api: apiRoot } : undefined,
});
```

**`messageCreate` handler:**

Filter logic (apply before any processing):
- Skip messages where `message.author.bot === true`
- If `guild_ids` is configured, skip messages where `message.guildId` is not in the list
- If `channel_ids` is configured, skip messages where `message.channelId` is not in the list

Message content extraction:
- `message.content` is the text body (empty string if message has only attachments)
- `message.attachments` is a `Collection<string, Attachment>` — iterate with `.values()`
- Each `Attachment` has: `.url` (CDN URL, publicly accessible), `.name`, `.contentType` (may be null), `.size`
- If `message.content` is empty and there are no attachments, skip the message (log it)

`extra_data` to populate:
```typescript
extra_data: {
   thread_id: message.channel.isThread() ? message.channelId : undefined,
  guild_id: message.guildId ?? undefined,
  channel_name: message.channel.isTextBased() && 'name' in message.channel
    ? message.channel.name
    : undefined,
}
```

`chat_id`: Use `message.channelId` (the channel snowflake). This is the stable identifier for a conversation.

`message_id`: Use `message.id` (snowflake string).

`reply_to_message_id`: If `message.reference?.messageId` exists, use it.

`from`:
```typescript
from: {
  id: message.author.id,
  username: message.author.username,
  display_name: message.member?.displayName ?? message.author.displayName,
}
```

File handling: Discord CDN URLs are publicly accessible without auth headers. Pass them directly as `url` in the `files` array. Use `attachment.contentType ?? "application/octet-stream"` for `mime_type`.

### 3. Outbound (Gateway → Discord)

**`POST /send` handler** receives a `SendRequest` body. The `sendOutbound()` function must:

1. Resolve the channel: `await client.channels.fetch(body.chat_id)` — throws if not found or not a text channel. Cast to `TextChannel | DMChannel`.

2. Handle `reply_to_message_id`: If present, fetch the message with `channel.messages.fetch(reply_to_message_id)` and use `message.reply()` instead of `channel.send()`. If the fetch fails (message deleted), fall back to `channel.send()` without reply.

3. Text-only send:
   ```typescript
   const sent = await channel.send({
     content: text,
     messageReference: replyMessage ? { messageId: replyMessage.id } : undefined,
   });
   ```

4. File sends: Read each file path from disk (validate with `validateFilePath` — copy from Telegram). Build an `AttachmentBuilder` array:
   ```typescript
   import { AttachmentBuilder } from "discord.js";
   const attachment = new AttachmentBuilder(buffer, { name: filename });
   ```
   Send all files in one call with `channel.send({ content: text, files: attachments })`. Discord supports up to 10 files per message and 25MB total. If there are more than 10 files, batch into multiple sends; return the last message ID.

5. Return `{ protocol_message_id: sent.id }`.

`extra_data` consumption: The outbound handler can optionally read `body.extra_data?.thread_id` to send into a thread instead of the parent channel. If `thread_id` is present, fetch that channel ID instead of `chat_id`.

Rate limits: Discord allows 5 `send` calls per 5 seconds per channel. discord.js has a built-in rate limit queue — do not implement manual throttling. The `retry()` helper handles transient 429s if discord.js surfaces them as errors.

### 4. Platform-Specific Considerations

**Privileged intents:** The `MESSAGE_CONTENT` intent must be enabled in the Discord Developer Portal under the bot's settings. Without it, `message.content` will be an empty string for messages in servers with 100+ members. Document this clearly in the config example.

**Snowflake IDs:** All Discord IDs are 64-bit integers represented as strings. Never parse them as `number` — JavaScript loses precision. Always keep them as strings.

**DM support:** The `Partials.Channel` partial is required to receive DMs before the channel is cached. Without it, DM `messageCreate` events are silently dropped.

**Scope:** Start with text messages and file attachments only. Skip embeds, stickers, reactions, and components — log them as unsupported and move on.

**Token login:** `client.login(CREDENTIAL_TOKEN)` is async and must be awaited in `main()`. The `ready` event fires when the bot is connected and ready to receive events.

**Reconnection:** discord.js handles reconnection automatically. No manual reconnect logic needed.

**E2E testing approach:** Mocking the Discord Gateway WebSocket is impractical. Instead, abstract the discord.js client behind a thin `DiscordClient` interface with methods `sendMessage()`, `fetchChannel()`, etc. In tests, inject a mock implementation. For REST-level testing, the `api_root` config field redirects REST calls to a mock HTTP server (same pattern as Telegram's `api_root`).

## E2E Testing

### Mock Server

The mock server needs to simulate Discord's REST API endpoints that discord.js calls:

- `GET /api/v10/gateway/bot` — returns `{"url": "wss://...", "shards": 1, "session_start_limit": {...}}`
- `GET /api/v10/users/@me` — returns a bot user object
- `POST /api/v10/channels/{channel_id}/messages` — records the call, returns a message object with a generated snowflake ID
- `GET /api/v10/channels/{channel_id}/messages/{message_id}` — returns a stored message or 404

Because discord.js opens a real WebSocket to the Discord Gateway, the E2E test should use the interface-injection approach: the `main.ts` exports a `createAdapter(client: DiscordClient)` factory, and tests pass a mock client directly. The Fastify server is still started on a real port.

Alternatively, use the `api_root` config field to point discord.js REST calls at the mock server, and trigger inbound events by calling a test-only endpoint on the adapter that fires the `messageCreate` handler directly.

### Scenarios

```gherkin
Feature: Discord adapter inbound

  Scenario: Text message forwarded to gateway
    Given the Discord adapter is running
    And the mock gateway is listening
    When a Discord messageCreate event fires with text "hello from discord"
    Then the adapter POSTs to /api/v1/adapter/inbound
    And the payload contains chat_id equal to the channel snowflake
    And the payload contains text "hello from discord"
    And the payload contains from.id equal to the author snowflake

  Scenario: Message with attachment forwarded to gateway
    Given the Discord adapter is running
    And the mock gateway is listening
    When a Discord messageCreate event fires with one attachment
    Then the adapter POSTs to /api/v1/adapter/inbound
    And the payload files array contains one entry with the attachment URL

  Scenario: Bot message is ignored
    Given the Discord adapter is running
    When a Discord messageCreate event fires from a bot user
    Then the adapter does NOT POST to /api/v1/adapter/inbound

  Scenario: Message filtered by guild_id
    Given the Discord adapter is running with guild_ids configured
    When a Discord messageCreate event fires from a non-configured guild
    Then the adapter does NOT POST to /api/v1/adapter/inbound

Feature: Discord adapter outbound

  Scenario: Text message sent to channel
    Given the Discord adapter is running
    And the mock Discord REST API is listening
    When the gateway POSTs to /send with chat_id and text "hello discord"
    Then the adapter calls POST /channels/{chat_id}/messages
    And the response contains protocol_message_id

  Scenario: File sent to channel
    Given the Discord adapter is running
    And a temp file exists at a valid absolute path
    When the gateway POSTs to /send with file_paths containing that path
    Then the adapter calls POST /channels/{chat_id}/messages with multipart form data
    And the response contains protocol_message_id

  Scenario: Reply to existing message
    Given the Discord adapter is running
    When the gateway POSTs to /send with reply_to_message_id set
    Then the adapter sends the message with a message_reference to that ID
```

### Test Gateway Integration

In `tests/test-gateway.ts`, add a `startWithDiscord()` method following the same pattern as `startWithTelegram()`:

```typescript
async startWithDiscord(options: {
  guildIds?: string[];
  channelIds?: string[];
}): Promise<{ mockDiscordApi: MockDiscordApi }> {
  const mockDiscordApi = new MockDiscordApi();
  await mockDiscordApi.start();

  await this.addCredential("discord_test", {
    adapter: "discord",
    token: "Bot test-token",
    active: true,
    config: {
      guild_ids: options.guildIds,
      channel_ids: options.channelIds,
      api_root: mockDiscordApi.baseUrl,
    },
  });

  return { mockDiscordApi };
}
```

`MockDiscordApi` is a Fastify server that records REST calls and lets tests assert on them. It also exposes a `triggerInbound(event)` method that calls the adapter's internal event handler directly (via a test-only endpoint or by exporting the handler).

## Config Example

Add to `config.example.json` under `credentials`:

```json
"my_discord": {
  "adapter": "discord",
  "token": "${DISCORD_BOT_TOKEN}",
  "active": true,
  "config": {
    "guild_ids": ["123456789012345678"],
    "channel_ids": ["987654321098765432"]
  },
  "route": {
    "channel": "discord"
  }
}
```

Note: `DISCORD_BOT_TOKEN` should be the raw bot token (without the `Bot ` prefix — discord.js strips it automatically if present).

## Checklist

- [ ] Create `adapters/discord/adapter.json`
- [ ] Create `adapters/discord/package.json` with discord.js ^14 and fastify ^5
- [ ] Create `adapters/discord/tsconfig.json` (copy from Telegram)
- [ ] Create `adapters/discord/src/main.ts`
  - [ ] Parse env vars and `CREDENTIAL_CONFIG`
  - [ ] Implement `log()` and `retry()` helpers
  - [ ] Define `InboundPayload` and `SendRequest` interfaces
  - [ ] Set up discord.js `Client` with correct intents (including `MessageContent`)
  - [ ] Implement `forwardToGateway()` (copy from Telegram)
  - [ ] Implement `messageCreate` handler with guild/channel filtering
  - [ ] Implement file attachment extraction from `message.attachments`
  - [ ] Implement `sendOutbound()` with text, file, and reply support
  - [ ] Implement `validateFilePath()` (copy from Telegram)
  - [ ] Set up Fastify with `GET /health` and `POST /send`
  - [ ] Implement `shutdown()` with SIGTERM/SIGINT handlers
  - [ ] Implement `main()` with `client.login()` and ready event
- [ ] Run `npm run build` — zero TypeScript errors
- [ ] Add `api_root` config support for E2E test redirection
- [ ] Write E2E mock server (`MockDiscordApi`) in `tests/`
- [ ] Write E2E scenarios: inbound text, inbound attachment, bot filter, guild filter, outbound text, outbound file, outbound reply
- [ ] Add `startWithDiscord()` to `tests/test-gateway.ts`
- [ ] Add credential example to `config.example.json`
- [ ] Verify `cargo test` still passes (no Rust changes, but confirm)
- [ ] Document `MESSAGE_CONTENT` privileged intent requirement in a comment in `main.ts`
