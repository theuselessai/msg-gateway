# msg-gateway Roadmap

## Vision

msg-gateway is the unified messaging layer for Pipelit and other LLM-based applications. It abstracts away protocol-specific complexity, providing a single, consistent interface for all messaging channels.

## Current Status: v0.3.0 (Released)

### v0.1.0
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

### v0.2.0
- Message format redesign with multi-file support
- E2E test framework (Cucumber-JS + BDD)
- Telegram adapter migrated to Node.js
- Named backends with per-credential routing
- CEL-based message guardrails with hot-reload
- OpenCode SSE response delivery + file attachments
- External backend subprocess protocol
- `plit` command-line client

### v0.3.0
- Full Pipelit integration — gateway is Pipelit's unified messaging layer (Pipelit PR #135)
- End-to-end protocol verification with real Pipelit instance

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

## v0.3.0 - Pipelit Integration & Production Hardening ✅ Released

### Goals
1. Full integration with Pipelit unified inbound endpoint
2. End-to-end protocol verification with real Pipelit instance

### Tasks

| Order | Task | Priority | Related | Status |
|-------|------|----------|---------|--------|
| 1 | `plit` — project setup, client, commands, REPL | P0 | #43 | ✅ Done (shipped in v0.2.0) |
| 2 | Pipelit unified inbound endpoint | P0 | Pipelit #134 | ✅ Done (Pipelit PR #135, merged Mar 11 2026) |
| 3 | Protocol verification & E2E testing | P0 | #8 | ✅ Done (manual + integration verified) |

Dev plan: [`docs/dev-plans/plit.md`](dev-plans/plit.md)

## v0.4.0 - Adapters & Distribution (Target: Jul 2026)

### Tasks

| Order | Issue | Task | Priority | Status |
|-------|-------|------|----------|--------|
| 1 | #34 | Full OpenCode server mode integration | P1 | Planned |
| 2 | #44 | Publish to crates.io (`cargo install plit-gw` / `plit`) | P2 | Planned |
| 3 | #45 | Separate adapters/backends into standalone repos | P2 | Planned |
| 4 | #46 | `plit install` — adapter/backend plugin management | P2 | Planned |
| 5 | #9 | Discord adapter | P2 | Planned |
| 6 | #10 | Slack adapter | P2 | Planned |
| 7 | #11 | Email adapter | P2 | Planned |

## v0.5.0 - `plit` CLI Ecosystem (Target: Sep 2026)

Rebrand the CLI as `plit` — the single entry point to the Pipelit ecosystem. Gateway becomes `plit-gw`, a managed component.

### Naming

```
crates.io: plit        → binary: plit       (ecosystem CLI)
crates.io: plit-gw     → binary: plit-gw    (gateway server)
PyPI:      pipelit     → unchanged          (workflow engine)
```

### Tasks

| Order | Issue | Task | Priority | Depends on | Status |
|-------|-------|------|----------|------------|--------|
| 1 | Pipelit#136 | User roles and RBAC (admin/normal/agent) | P0 | — | Planned |
| 2 | Pipelit#137 | User management API (create, list, delete) | P0 | Pipelit#136 | Planned |
| 3 | Pipelit#138 | Gateway ↔ Pipelit instance pairing | P0 | — | Planned |
| 4 | #47 | Rename CLI to `plit`, gateway to `plit-gw` | P0 | — | Planned |
| 5 | #48 | `plit init` — setup wizard (prereq checks, config gen, first user) | P0 | Pipelit#136-138, #47 | Planned |
| 6 | #49 | `plit start` — process supervisor (honcho) | P1 | #48 | Planned |

### Open decisions (must resolve before dev plans)
- Role model: enum on UserProfile vs separate Role table?
- Permission scope: role-based only, or per-resource ACLs?
- Pairing: manual tokens, semi-auto (`plit init`), or full auto-registration?
- Agent permissions: which endpoints, can agents create agents?
- First user: `plit init` creates admin, or separate `plit user create`?

## Future Considerations (v0.6.0+)

### Production Hardening
- Rate limiting
- Metrics & observability (Prometheus)

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
