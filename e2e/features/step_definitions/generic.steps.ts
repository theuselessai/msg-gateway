import { Given, When, Then } from '@cucumber/cucumber';
import { expect } from 'chai';
import { TestWorld } from './world';
import { MockFileServer } from '../../support/mock-file-server';

Given('a mock file server running', async function (this: TestWorld) {
  if (!this.mockFileServer) {
    this.mockFileServer = new MockFileServer();
    await this.mockFileServer.start();
  }
});

When(
  'I POST {string} with body from mock file server',
  async function (this: TestWorld, path: string) {
    expect(this.mockFileServer).to.not.be.null;
    const body = JSON.stringify({
      chat_id: 'chat_file_test',
      text: 'File test message',
      from: { id: 'user_file' },
      files: [
        {
          url: this.mockFileServer!.fileUrl,
          filename: 'test.txt',
          mime_type: 'text/plain',
        },
      ],
    });
    this.lastResponse = await this.gatewayFetch(path, {
      method: 'POST',
      body,
    });
    try {
      this.lastResponseBody = await this.lastResponse.clone().json();
    } catch {
      this.lastResponseBody = null;
    }
  }
);

Then(
  'the received message should have {int} attachment(s)',
  function (this: TestWorld, count: number) {
    const msg = this.lastBackendMessage as Record<string, unknown>;
    const attachments = msg.attachments as unknown[];
    expect(attachments).to.be.an('array');
    expect(attachments).to.have.length(count);
  }
);

Then(
  'the attachment filename should be {string}',
  function (this: TestWorld, expected: string) {
    const msg = this.lastBackendMessage as Record<string, unknown>;
    const attachments = msg.attachments as Array<Record<string, unknown>>;
    expect(attachments[0].filename).to.equal(expected);
  }
);

Then(
  'the attachment mime_type should be {string}',
  function (this: TestWorld, expected: string) {
    const msg = this.lastBackendMessage as Record<string, unknown>;
    const attachments = msg.attachments as Array<Record<string, unknown>>;
    expect(attachments[0].mime_type).to.equal(expected);
  }
);
