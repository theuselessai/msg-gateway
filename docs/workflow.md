# Development Workflow

This document defines how issues move through the project board and what each status means.

## Issue Lifecycle

```
Backlog ──(dev plan merged)──▶ Ready ──(PR opened)──▶ In Review ──(PR merged)──▶ Done
                                                                                      │
                                         ◀──────────────────────────────────────────(unblock next)
```

## Statuses

### Backlog
Work that is not yet ready to start. Either blocked by another issue, or missing a dev plan.

### Ready
Work that can be picked up immediately. **Requirements:**
- All blocking issues are resolved
- A dev plan exists (see [Dev Plans](#dev-plans) below)

### In Review
A PR is open for this issue. The issue stays here until:
- All CI checks pass (Lint, Test, Build, E2E)
- All gito.bot review comments are triaged and valid ones addressed
- A human approves and merges the PR

### Done
The PR is merged to master.

## Dev Plans

A **dev plan** is required before an issue can move to Ready. It answers the questions a developer needs before writing code.

**Location:** `docs/dev-plans/{issue-slug}.md`

**For adapter issues**, a dev plan must cover:
- Library choices (e.g. which npm package for the platform API)
- Auth mechanism (bot token, OAuth, IMAP credentials, etc.)
- `extra_data` fields populated on inbound and consumed on outbound
- File attachment handling
- Any platform-specific quirks or limitations

**For core feature issues**, a dev plan must cover:
- Acceptance criteria
- Approach and key design decisions
- Files affected and how
- Open questions

Dev plans go through a PR like any other change — they get reviewed before the issue moves to Ready.

**Exception:** Issues where the existing documentation (e.g. `docs/adapters/protocol.md`, issue description, or prior art in the codebase) is sufficient to start work immediately do not need a separate dev plan file. This is a judgment call made at the time of triaging.

## Transitions (Atlas responsibilities)

### Backlog → Ready
- Confirm all blocking issues are Done on the board
- Confirm a dev plan exists (or sufficient documentation as above)
- Remove the `blocked` label from the issue
- Move item to Ready on the project board

### Ready → In Review
- Happens when a PR is opened that references the issue (`Closes #N` in PR body)
- Move item to In Review on the project board

### In Review → Done
- Happens when the PR is merged
- Move item to Done on the project board
- Check the roadmap dependency graph for newly unblocked issues
- For each unblocked issue: if dev plan exists → move to Ready, else leave in Backlog

## PR Rules

- **Never auto-merge.** Always create the PR, report the URL, and wait for a human to merge.
- PRs must reference their issue via `Closes #N` in the body.
- One issue per PR where possible.
- Dev plan PRs are merged before implementation PRs.

## Project Board

- **Project:** [pipelit](https://github.com/orgs/theuselessai/projects/1)
- **Project ID:** `PVT_kwDODB9zRs4BOzYC`
- **Status field ID:** `PVTSSF_lADODB9zRs4BOzYCzg9Za8o`
  - Ready: `61e4505c`
  - Backlog: `f75ad846`
  - In progress: `47fc9ee4`
  - Done: `98236657`
