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
    void _err;
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
    void _err;
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
    await new Promise<void>((resolve, reject) => {
      const ws = new WebSocket(wsUrl, { headers: { Authorization: `Bearer ${token}` } });
      ws.on('open', () => {
        this.wsClient = ws;
        resolve();
      });
      ws.on('error', reject);
      setTimeout(() => reject(new Error('WebSocket connection timed out')), 5000);
    });
  }
);

Then(
  'the WebSocket client should receive a message with text {string} within {int}ms',
  async function (this: TestWorld, expectedText: string, timeoutMs: number) {
    const ws = this.wsClient!;
    await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(
        () => reject(new Error(`WS message not received within ${timeoutMs}ms`)),
        timeoutMs
      );
      ws.on('message', (data: WebSocket.RawData) => {
        clearTimeout(timer);
        const msg = JSON.parse(data.toString()) as Record<string, unknown>;
        expect(msg.text).to.equal(expectedText);
        resolve();
      });
    });
  }
);
