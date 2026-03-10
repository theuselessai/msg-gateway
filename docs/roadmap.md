# msg-gateway Roadmap

## Vision

msg-gateway is the unified messaging layer for Pipelit and other LLM-based applications. It abstracts away protocol-specific complexity, providing a single, consistent interface for all messaging channels.

## Current Status: v0.1.0 (Released)

- Core gateway with HTTP server (Axum)
- Generic adapter (built-in REST + WebSocket)
- Telegram adapter (Python, external process)
- Pipelit backend protocol support
- OpenCode backend protocol support
- Config hot reload
- Health monitoring with message buffering
- File caching for attachments
- Admin API for credential management
- ~80% test coverage

## v0.2.0 - Foundation & E2E Testing (Target: Apr 2026)

### Goals
1. Establish E2E testing framework with BDD/Gherkin
2. Migrate Telegram adapter to Node.js (no virtualenv dependency)
3. Message format redesign with file support
4. Named backends with per-credential routing
5. CEL-based message guardrails with hot-reload
6. OpenCode backend adapter (built-in + external)

### Tasks

| Order | Issue | Task | Priority | Status |
|-------|-------|------|----------|--------|
| 1 | #15 | Message Format Redesign (core fields, files[], extra_data) | P0 | ✅ Done |
| 2 | #16 | File Upload API (POST /api/v1/files) | P0 | ✅ Done |
| 3 | #12 | E2E Test Framework (Cucumber-JS) | P0 | ✅ Done |
| 4 | #13 | Telegram Adapter → Node.js | P0 | ✅ Done |
| 5 | #17 | Generic Adapter File Support | P1 | ✅ Done |
| 6 | #29 | Complete E2E Test Coverage (19 scenarios) | P1 | ✅ Done |
| 7 | #32 | OpenCode Backend Adapter (built-in Rust) | P1 | ✅ Done |
| 8 | #33 | External Backend Subprocess Protocol | P1 | ✅ Done |
| 9 | #38 | Named Backends with Per-Credential Routing | P1 | ✅ Done |
| 10 | #40 | CEL-Based Guardrails with Hot-Reload | P1 | ✅ Done |
| 11 | #41 | OpenCode SSE Response Delivery + File Attachments | P1 | ✅ Done |
| 12 | — | Protocol Documentation | P2 | Planned |

### Technical Decisions

#### E2E Testing Stack
- **Framework**: Cucumber-JS (@cucumber/cucumber)
- **Language**: TypeScript
- **HTTP Client**: undici (Node.js built-in)
- **Assertions**: chai
- **Reason**: BDD with Gherkin provides readable, living documentation

#### Adapter Stack
- **Language**: Node.js (TypeScript)
- **HTTP Framework**: Fastify (lightweight, fast)
- **Reason**: No virtualenv needed, fast startup, consistent tooling

## v0.3.0 - CLI Tool & Pipelit Integration (Target: May 2026)

### Goals
1. `gw` CLI tool — unix-philosophy gateway client for chat, admin, and agent tooling
2. Full integration with Pipelit unified inbound endpoint
3. End-to-end protocol verification with real Pipelit instance
4. Production hardening (rate limiting, observability)

### Tasks

| Order | Task | Priority | Related | Status |
|-------|------|----------|---------|--------|
| 1 | `gw` CLI — project setup & client foundation | P0 | — | Planned |
| 2 | `gw send` + `gw listen` + `gw health` commands | P0 | — | Planned |
| 3 | `gw chat` interactive REPL | P0 | — | Planned |
| 4 | `gw credentials` admin commands | P1 | — | Planned |
| 5 | Pipelit unified inbound endpoint | P0 | Pipelit #134 | Planned |
| 6 | Protocol verification & E2E testing | P0 | #8 | Planned |
| 7 | Rate limiting | P2 | — | Planned |
| 8 | Metrics & observability (Prometheus) | P2 | — | Planned |

Dev plan: [`docs/dev-plans/gw-cli.md`](dev-plans/gw-cli.md)

### OpenCode Enhancements (parallel track)

| Issue | Task | Priority | Status |
|-------|------|----------|--------|
| #35 | Async message mode for OpenCode backend | P2 | Open |
| #34 | Full OpenCode server mode integration | P2 | Open |

## Future Considerations (v0.4.0+)

### Additional Adapters (deprioritized from v0.2.0)

| Issue | Adapter | Estimate | Notes |
|-------|---------|----------|-------|
| #11 | Email (IMAP/SMTP) | 5-7 days | Dev plan in docs/dev-plans/email-adapter.md |
| #10 | Slack (Events API) | 3-5 days | Dev plan in docs/dev-plans/slack-adapter.md |
| #9 | Discord (discord.js) | 3-5 days | Dev plan in docs/dev-plans/discord-adapter.md |

### Other
- WhatsApp adapter
- Microsoft Teams adapter
- Matrix adapter
- Webhook adapter (generic inbound webhooks)
- Message transformation plugins
- Multi-tenant support
- **Per-credential backend isolation**: Current backend model is singleton-per-name (one process shared
  across all credentials pointing to the same backend). This breaks when credentials need different
  upstream endpoints or isolated accounts (e.g. each user has their own OpenCode instance, or
  per-credential Pipelit tokens). Fix: spawn one backend subprocess per credential, using
  `CREDENTIAL_CONFIG` to carry per-credential config — same pattern as external adapters.
  See `docs/architecture.md` § Backend Scaling Model.

## Related Projects

- [Pipelit](https://github.com/theuselessai/Pipelit) - LLM workflow platform
- Pipelit Project Board: https://github.com/orgs/theuselessai/projects/1

## Contributing

See [CONTRIBUTING.md](../CONTRIBUTING.md) for development guidelines.
