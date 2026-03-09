import { Given, When, Then } from '@cucumber/cucumber';
import { expect } from 'chai';
import WebSocket from 'ws';
import { TestWorld } from './world';
import { MockOpencodeServer } from '../../support/mock-opencode-server';

const OPENCODE_CHAT_ID = 'opencode-chat-1';

Given('a gateway configured with OpenCode backend', async function (this: TestWorld) {
  this.mockOpencodeServer = new MockOpencodeServer();
  const opencodePort = await this.mockOpencodeServer.start();
  this.gateway = await (async () => {
    const { TestGateway } = await import('../../support/test-gateway');
    const gw = new TestGateway();
    await gw.startWithOpencodeConfig(opencodePort);
    return gw;
  })();
});

Given('a WebSocket client connected to the OpenCode generic adapter', async function (this: TestWorld) {
  if (!this.gateway) throw new Error('Gateway not initialized');
  const wsUrl = this.gateway.gatewayUrl.replace('http://', 'ws://') + `/ws/chat/test_opencode/${OPENCODE_CHAT_ID}`;
  this.wsMessages = [];
  await new Promise<void>((resolve, reject) => {
    const ws = new WebSocket(wsUrl, { headers: { Authorization: 'Bearer generic_token' } });
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
});

When('I send a message {string} via the OpenCode WebSocket', async function (this: TestWorld, text: string) {
  if (!this.gateway) throw new Error('Gateway not initialized');
  const response = await fetch(
    `${this.gateway.gatewayUrl}/api/v1/chat/test_opencode`,
    {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Authorization': 'Bearer generic_token',
      },
      body: JSON.stringify({ chat_id: OPENCODE_CHAT_ID, text, from: { id: 'user_opencode' } }),
    }
  );
  expect(response.status).to.equal(202);
});

Then('the OpenCode mock should receive the message', async function (this: TestWorld) {
  if (!this.mockOpencodeServer) throw new Error('MockOpencodeServer not initialized');
  await this.mockOpencodeServer.waitForMessage(8000);
});

Then('the WebSocket client should receive the OpenCode AI response', async function (this: TestWorld) {
  const msg = (await this.waitForWsMessage(8000)) as Record<string, unknown>;
  expect(msg.text).to.equal('E2E mock AI response');
});

Then('the OpenCode mock should have created exactly {int} session', async function (this: TestWorld, count: number) {
  if (!this.mockOpencodeServer) throw new Error('MockOpencodeServer not initialized');
  const deadline = Date.now() + 10000;
  while (this.mockOpencodeServer.getSessionRequests().length < count && Date.now() < deadline) {
    await new Promise((r) => setTimeout(r, 100));
  }
  expect(this.mockOpencodeServer.getSessionRequests()).to.have.length(count);
});

Then('the OpenCode mock should have received {int} messages', async function (this: TestWorld, count: number) {
  if (!this.mockOpencodeServer) throw new Error('MockOpencodeServer not initialized');
  const deadline = Date.now() + 10000;
  while (this.mockOpencodeServer.getMessages().length < count && Date.now() < deadline) {
    await new Promise((r) => setTimeout(r, 100));
  }
  expect(this.mockOpencodeServer.getMessages()).to.have.length(count);
});
