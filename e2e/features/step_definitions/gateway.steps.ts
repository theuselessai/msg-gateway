import { Given, When, Then, defineStep } from '@cucumber/cucumber';
import { expect } from 'chai';
import WebSocket from 'ws';
import { TestWorld } from './world';
import { TestGateway } from '../../support/test-gateway';
import { MockBackend } from '../../support/mock-backend';

Given('a running gateway', async function (this: TestWorld) {
  this.backend = new MockBackend();
  await this.backend.start();
  this.gateway = new TestGateway();
  await this.gateway.start(this.backend);
});

Given('a mock backend listening', async function (this: TestWorld) {
  if (!this.backend) {
    this.backend = new MockBackend();
    await this.backend.start();
  }
  this.backend.reset();
});

When('I GET {string}', async function (this: TestWorld, path: string) {
  this.lastResponse = await this.gatewayFetch(path, { method: 'GET' });
  try {
    this.lastResponseBody = await this.lastResponse.clone().json();
  } catch (_err) {
    this.lastResponseBody = null;
  }
});

When('I POST {string} with body:', async function (this: TestWorld, path: string, body: string) {
  this.lastResponse = await this.gatewayFetch(path, {
    method: 'POST',
    body: body.trim(),
  });
  try {
    this.lastResponseBody = await this.lastResponse.clone().json();
  } catch (_err) {
    this.lastResponseBody = null;
  }
});

defineStep('header {string} is {string}', function (this: TestWorld, name: string, value: string) {
  this.pendingHeaders[name] = value;
});

Then('the response status should be {int}', function (this: TestWorld, expectedStatus: number) {
  expect(this.lastResponse).to.not.be.null;
  expect(this.lastResponse!.status).to.equal(expectedStatus);
});

Then(
  'the backend should receive a message within {int}ms',
  async function (this: TestWorld, timeoutMs: number) {
    this.lastBackendMessage = await this.backend!.waitForMessage(timeoutMs);
  }
);

Then(
  'the received message text should be {string}',
  function (this: TestWorld, expected: string) {
    const msg = this.lastBackendMessage as Record<string, unknown>;
    expect(msg.text).to.equal(expected);
  }
);

Then(
  'the received message credential_id should be {string}',
  function (this: TestWorld, expected: string) {
    const msg = this.lastBackendMessage as Record<string, unknown>;
    expect(msg.credential_id).to.equal(expected);
  }
);

Then(
  'the received message source.protocol should be {string}',
  function (this: TestWorld, expected: string) {
    const msg = this.lastBackendMessage as Record<string, unknown>;
    const source = msg.source as Record<string, unknown>;
    expect(source.protocol).to.equal(expected);
  }
);

Given(
  'a WebSocket client connected to {string} with token {string}',
  async function (this: TestWorld, wsPath: string, token: string) {
    const wsUrl = this.gateway!.gatewayUrl.replace('http://', 'ws://') + wsPath;
    this.wsMessages = [];
    await new Promise<void>((resolve, reject) => {
      const ws = new WebSocket(wsUrl, { headers: { Authorization: `Bearer ${token}` } });
      const timeoutId = setTimeout(() => {
        ws.off('open', openHandler);
        ws.off('error', errorHandler);
        ws.terminate();
        reject(new Error('WebSocket connection timed out'));
      }, 5000);
      const openHandler = () => {
        clearTimeout(timeoutId);
        ws.off('error', errorHandler);
        this.wsClient = ws;
        ws.on('message', (data: WebSocket.RawData) => {
          try {
            this.wsMessages.push(JSON.parse(data.toString()));
          } catch (_err) {}
        });
        resolve();
      };
      const errorHandler = (err: Error) => {
        clearTimeout(timeoutId);
        ws.off('open', openHandler);
        reject(err);
      };
      ws.on('open', openHandler);
      ws.on('error', errorHandler);
    });
  }
);

Then(
  'the WebSocket client should receive a message with text {string} within {int}ms',
  async function (this: TestWorld, expectedText: string, timeoutMs: number) {
    const msg = (await this.waitForWsMessage(timeoutMs)) as Record<string, unknown>;
    expect(msg.text).to.equal(expectedText);
  }
);

Then(
  'the response body status should be {string}',
  function (this: TestWorld, expected: string) {
    expect(this.lastResponseBody).to.not.be.null;
    const body = this.lastResponseBody as Record<string, unknown>;
    expect(body.status).to.equal(expected);
  }
);
