# E2E Testing Guide

This document describes the end-to-end testing strategy for msg-gateway.

## Overview

E2E tests verify the complete message flow:
```
Test Client → Adapter → Gateway → Mock Backend
                                       ↓
Test Client ← Adapter ← Gateway ← Mock Backend
```

## Technology Stack

| Component | Technology | Reason |
|-----------|------------|--------|
| Framework | Cucumber-JS | BDD with Gherkin syntax |
| Language | TypeScript | Type safety, matches adapters |
| HTTP Client | undici | Node.js built-in, fast |
| Assertions | chai | Rich assertion API |
| Process Management | execa | Spawn gateway & adapters |

## Directory Structure

```
e2e/
├── package.json
├── tsconfig.json
├── cucumber.js                 # Cucumber configuration
├── features/
│   ├── gateway.feature         # Core gateway scenarios
│   ├── generic.feature         # Generic adapter scenarios
│   ├── telegram.feature        # Telegram adapter scenarios
│   └── step_definitions/
│       ├── gateway.steps.ts
│       ├── adapters.steps.ts
│       └── world.ts            # Shared test context
├── support/
│   ├── mock-backend.ts         # Mock Pipelit backend
│   ├── test-gateway.ts         # Gateway process management
│   └── test-adapter.ts         # Adapter process management
└── fixtures/
    └── config.json             # Test configuration
```

## Feature Examples

### Gateway Core

```gherkin
# features/gateway.feature
Feature: Gateway Message Routing

  Background:
    Given a running msg-gateway
    And a mock Pipelit backend

  Scenario: Route inbound message to backend
    Given a Generic adapter credential "test_generic"
    When a user sends message "Hello" to chat "chat_001"
    Then the backend should receive a message
    And the message text should be "Hello"
    And the message credential_id should be "test_generic"
    And the message source.protocol should be "generic"

  Scenario: Route outbound message to user
    Given a Generic adapter credential "test_generic"
    And a WebSocket client connected to "test_generic" chat "chat_001"
    When the backend sends reply "Hi there" to credential "test_generic" chat "chat_001"
    Then the WebSocket client should receive message "Hi there"

  Scenario: Buffer messages when backend is down
    Given the backend is unreachable
    And a Generic adapter credential "test_generic"
    When a user sends message "Buffered" to chat "chat_001"
    Then the message should be buffered
    When the backend becomes available
    Then the buffered message should be delivered
```

### Generic Adapter

```gherkin
# features/generic.feature
Feature: Generic Adapter

  Background:
    Given a running msg-gateway
    And a mock Pipelit backend
    And a Generic adapter credential "test_generic" with token "test_token"

  Scenario: REST inbound message
    When I POST to "/api/v1/chat/test_generic" with:
      """
      {
        "chat_id": "chat_001",
        "text": "Hello via REST",
        "from": {"id": "user_001"}
      }
      """
    And header "Authorization" is "Bearer test_token"
    Then the response status should be 202
    And the backend should receive the message

  Scenario: WebSocket outbound message
    Given a WebSocket connection to "/ws/chat/test_generic/chat_001"
    When the backend sends:
      """
      {
        "credential_id": "test_generic",
        "chat_id": "chat_001",
        "text": "Hello via WebSocket"
      }
      """
    Then the WebSocket should receive:
      """
      {
        "text": "Hello via WebSocket"
      }
      """

  Scenario: Authentication required
    When I POST to "/api/v1/chat/test_generic" with:
      """
      {"chat_id": "chat_001", "text": "No auth"}
      """
    Then the response status should be 401
```

### External Adapter

```gherkin
# features/telegram.feature
Feature: Telegram Adapter

  Background:
    Given a running msg-gateway
    And a mock Pipelit backend
    And a Telegram adapter credential "test_telegram"

  Scenario: Receive message from Telegram
    When the Telegram adapter receives a message:
      | chat_id    | 123456789         |
      | message_id | tg_001            |
      | text       | Hello from Telegram |
      | from_id    | user_789          |
    Then the backend should receive a message
    And the message source.protocol should be "telegram"
    And the message source.chat_id should be "123456789"

  Scenario: Send reply to Telegram
    When the backend sends reply to credential "test_telegram":
      | chat_id | 123456789       |
      | text    | Reply to Telegram |
    Then the Telegram adapter /send should be called
    And the send request chat_id should be "123456789"
    And the send request text should be "Reply to Telegram"
```

## Step Definitions

### World (Shared Context)

```typescript
// features/step_definitions/world.ts
import { World, setWorldConstructor } from '@cucumber/cucumber';
import { MockBackend } from '../../support/mock-backend';
import { TestGateway } from '../../support/test-gateway';

export class TestWorld extends World {
  gateway?: TestGateway;
  backend?: MockBackend;
  wsClients: Map<string, WebSocket> = new Map();
  lastResponse?: Response;
  
  async startGateway(config: any) {
    this.gateway = new TestGateway(config);
    await this.gateway.start();
  }
  
  async startBackend() {
    this.backend = new MockBackend();
    await this.backend.start();
  }
  
  async cleanup() {
    await this.gateway?.stop();
    await this.backend?.stop();
    for (const ws of this.wsClients.values()) {
      ws.close();
    }
  }
}

setWorldConstructor(TestWorld);
```

### Gateway Steps

```typescript
// features/step_definitions/gateway.steps.ts
import { Given, When, Then, After } from '@cucumber/cucumber';
import { expect } from 'chai';
import { TestWorld } from './world';

Given('a running msg-gateway', async function(this: TestWorld) {
  await this.startGateway({
    gateway: {
      listen: '127.0.0.1:0', // Random port
      admin_token: 'test_admin',
    },
    auth: { send_token: 'test_send' },
    credentials: {},
  });
});

Given('a mock Pipelit backend', async function(this: TestWorld) {
  await this.startBackend();
});

Then('the backend should receive a message', async function(this: TestWorld) {
  const message = await this.backend!.waitForMessage(5000);
  expect(message).to.exist;
  this.lastMessage = message;
});

Then('the message text should be {string}', function(this: TestWorld, expected: string) {
  expect(this.lastMessage.text).to.equal(expected);
});

After(async function(this: TestWorld) {
  await this.cleanup();
});
```

## Running Tests

### Setup

```bash
cd e2e
npm install
```

### Run All Tests

```bash
npm test
# or
npx cucumber-js
```

### Run Specific Feature

```bash
npx cucumber-js features/gateway.feature
```

### Run by Tag

```bash
npx cucumber-js --tags "@smoke"
npx cucumber-js --tags "not @slow"
```

### Generate Report

```bash
npx cucumber-js --format html:reports/cucumber.html
```

## CI Integration

```yaml
# .github/workflows/ci.yml
e2e:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v5
    
    - name: Setup Node.js
      uses: actions/setup-node@v4
      with:
        node-version: '20'
        cache: 'npm'
        cache-dependency-path: e2e/package-lock.json
    
    - name: Setup Rust
      uses: dtolnay/rust-action@stable
    
    - name: Build gateway
      run: cargo build --release
    
    - name: Install E2E dependencies
      run: npm ci
      working-directory: e2e
    
    - name: Run E2E tests
      run: npm test
      working-directory: e2e
    
    - name: Upload report
      if: always()
      uses: actions/upload-artifact@v4
      with:
        name: e2e-report
        path: e2e/reports/
```

## Writing New Tests

1. **Add feature file** in `features/`
2. **Implement steps** in `features/step_definitions/`
3. **Add support code** if needed in `support/`
4. **Run locally** to verify
5. **Tag appropriately** (`@smoke`, `@slow`, etc.)

## Best Practices

1. **Independent scenarios**: Each scenario should be self-contained
2. **Background for setup**: Use Background for common Given steps
3. **Descriptive steps**: Steps should read like documentation
4. **Avoid coupling**: Don't share state between scenarios
5. **Fast feedback**: Tag slow tests, run @smoke in PR checks
6. **Clean up**: Always clean up resources in After hooks

## See Also

- [Cucumber-JS Documentation](https://cucumber.io/docs/installation/javascript/)
- [Architecture](../architecture.md)
- [Adapter Protocol](../adapters/protocol.md)
