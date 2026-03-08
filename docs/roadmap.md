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

| # | Task | Priority | Estimate | Status |
|---|------|----------|----------|--------|
| 1 | E2E Test Framework (Cucumber-JS) | P0 | 3-4 days | Planned |
| 2 | Telegram Adapter → Node.js | P0 | 2-3 days | Planned |
| 3 | Email Adapter (Node.js) | P1 | 5-7 days | Planned |
| 4 | Slack Adapter (Node.js) | P1 | 3-5 days | Planned |
| 5 | Discord Adapter (Node.js) | P1 | 3-5 days | Planned |
| 6 | Protocol Documentation | P1 | 1 day | Planned |

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
Week 1: E2E Framework + Telegram Migration
  └── Cucumber-JS setup with basic scenarios
  └── Telegram adapter rewritten in Node.js
  └── CI integration for E2E tests

Week 2-3: Email + Slack Adapters
  └── Email adapter (IMAP/SMTP)
  └── Slack adapter (Events API)
  └── E2E tests for both

Week 4: Discord + Documentation
  └── Discord adapter (discord.js)
  └── Protocol documentation complete
  └── v0.2.0 release
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
