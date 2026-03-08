import { Given, When, Then } from '@cucumber/cucumber';
import { expect } from 'chai';
import { TestWorld } from './world';
import { TestGateway } from '../../support/test-gateway';
import { MockBackend } from '../../support/mock-backend';
import { MockTelegramServer } from '../../support/mock-telegram-server';

Given('a running gateway with Telegram adapter', async function (this: TestWorld) {
  if (!this.backend) {
    this.backend = new MockBackend();
    await this.backend.start();
  }

  if (!this.mockTelegramServer) {
    this.mockTelegramServer = new MockTelegramServer();
    await this.mockTelegramServer.start();
  }

  this.gateway = new TestGateway();
  await this.gateway.startWithTelegram(this.backend, this.mockTelegramServer.url);
});

Given('a mock Telegram server running', async function (this: TestWorld) {
  if (!this.mockTelegramServer) {
    this.mockTelegramServer = new MockTelegramServer();
    await this.mockTelegramServer.start();
  }
});

When(
  'a Telegram user sends text {string} in chat {int}',
  async function (this: TestWorld, text: string, chatId: number) {
    expect(this.mockTelegramServer, 'mockTelegramServer is not initialized').to.exist;
    // Wait for adapter to start polling before injecting
    const deadline = Date.now() + 5000;
    while (Date.now() < deadline) {
      const log = this.mockTelegramServer!.requestLog;
      if (log.filter(m => m === 'getUpdates').length >= 2) break;
      await new Promise(r => setTimeout(r, 200));
    }
    if (this.mockTelegramServer!.requestLog.filter(m => m === 'getUpdates').length < 2) {
      throw new Error('Telegram adapter did not start polling within 5000ms');
    }
    this.mockTelegramServer!.injectTextMessage(chatId, text, {
      id: 99999,
      first_name: 'TestUser',
      username: 'testuser',
    });
  }
);

Then(
  'Telegram should receive a sendMessage within {int}ms',
  async function (this: TestWorld, timeoutMs: number) {
    expect(this.mockTelegramServer, 'mockTelegramServer is not initialized').to.exist;
    const msg = await this.mockTelegramServer!.waitForSentMessage(timeoutMs);
    this.lastTelegramSentMessage = msg as unknown as Record<string, unknown>;
  }
);

Then(
  'the Telegram message text should be {string}',
  function (this: TestWorld, expected: string) {
    expect(this.lastTelegramSentMessage, 'lastTelegramSentMessage is not set').to.exist;
    expect(this.lastTelegramSentMessage!.text).to.equal(expected);
  }
);

Then(
  'the Telegram message chat_id should be {string}',
  function (this: TestWorld, expected: string) {
    expect(this.lastTelegramSentMessage, 'lastTelegramSentMessage is not set').to.exist;
    expect(this.lastTelegramSentMessage!.chat_id).to.equal(expected);
  }
);
