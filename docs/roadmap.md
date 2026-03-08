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

## v0.2.0 - Adapters & E2E Testing (Target: Apr 2026)

### Goals
1. Establish E2E testing framework with BDD/Gherkin
2. Migrate all adapters to Node.js (no virtualenv dependency)
3. Add Email, Slack, Discord adapters
4. Complete protocol documentation

### Tasks

| Order | Issue | Task | Priority | Estimate | Blocked By | Status |
|-------|-------|------|----------|----------|------------|--------|
| 1 | #15 | Message Format Redesign (core fields, files[], extra_data) | P0 | 2-3 days | — | ✅ Done |
| 2 | #16 | File Upload API (POST /api/v1/files) | P0 | 1-2 days | #15 | ✅ Done |
| 3 | #12 | E2E Test Framework (Cucumber-JS) | P0 | 3-4 days | #15 | ✅ Done |
| 4 | #13 | Telegram Adapter → Node.js | P0 | 2-3 days | #15, #12 | ✅ Done |
| 5 | #17 | Generic Adapter File Support | P1 | 1-2 days | #15, #16 | ✅ Done |
| 6 | #11 | Email Adapter (Node.js) | P1 | 5-7 days | #15, #12 | Planned |
| 7 | #10 | Slack Adapter (Node.js) | P1 | 3-5 days | #15, #12 | Planned |
| 8 | #9 | Discord Adapter (Node.js) | P1 | 3-5 days | #15, #12 | Planned |
| 9 | — | Protocol Documentation | P1 | 1 day | — | Planned |

### Dependency Graph

```
#15 Message Format Redesign
 ├──▶ #16 File Upload API
 │     └──▶ #17 Generic Adapter File Support
 ├──▶ #12 E2E Test Framework
 │     ├──▶ #13 Telegram → Node.js
 │     ├──▶ #11 Email Adapter
 │     ├──▶ #10 Slack Adapter
 │     └──▶ #9  Discord Adapter
 └──▶ #8  Pipelit Integration (v0.3.0)
```

### Phases

**Phase 1 — Foundation** (`phase:1-foundation`)
- #15 → #16 → #17: Message format, file API, generic adapter files
- #12: E2E test framework (can run in parallel with #16)

**Phase 2 — Adapters** (`phase:2-adapters`)
- #13, #11, #10, #9: All adapter work (depends on Phase 1)

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

### Milestones

```
Week 1: Phase 1 — Foundation
  ├── #15 Message Format Redesign (Rust structs + server logic)
  ├── #16 File Upload API
  ├── #12 E2E Test Framework (Cucumber-JS setup + CI)
  └── #17 Generic Adapter File Support

Week 2: Phase 2a — Telegram Migration
  └── #13 Telegram Adapter → Node.js (first adapter on new format)

Week 3-4: Phase 2b — New Adapters
  ├── #11 Email Adapter (IMAP/SMTP)
  ├── #10 Slack Adapter (Events API)
  ├── #9  Discord Adapter (discord.js)
  └── Protocol Documentation
```

## v0.3.0 - Pipelit Integration (Target: May 2026)

### Goals
1. Full integration with Pipelit unified inbound endpoint
2. Multi-model routing support
3. Production hardening

### Tasks

| Task | Priority | Related |
|------|----------|---------|
| Pipelit unified inbound endpoint | P0 | Pipelit #134 |
| Protocol verification & testing | P0 | #8 |
| Multi-model routing (Claude/GLM/MiniMax) | P1 | Pipelit #126 |
| Rate limiting | P2 | - |
| Metrics & observability | P2 | - |

## Future Considerations (v0.4.0+)

- WhatsApp adapter
- Microsoft Teams adapter
- Matrix adapter
- Webhook adapter (generic inbound webhooks)
- Message transformation plugins
- Multi-tenant support

## Related Projects

- [Pipelit](https://github.com/theuselessai/Pipelit) - LLM workflow platform
- Pipelit Project Board: https://github.com/orgs/theuselessai/projects/1

## Contributing

See [CONTRIBUTING.md](../CONTRIBUTING.md) for development guidelines.
