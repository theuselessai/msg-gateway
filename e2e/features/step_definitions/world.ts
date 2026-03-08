import { World, setWorldConstructor, After, IWorldOptions, setDefaultTimeout } from '@cucumber/cucumber';
import { MockBackend } from '../../support/mock-backend';
import { MockTelegramServer } from '../../support/mock-telegram-server';
import { MockFileServer } from '../../support/mock-file-server';
import { TestGateway } from '../../support/test-gateway';
import WebSocket from 'ws';

setDefaultTimeout(30000);

export class TestWorld extends World {
  gateway: TestGateway | null = null;
  backend: MockBackend | null = null;
  mockTelegramServer: MockTelegramServer | null = null;
  mockFileServer: MockFileServer | null = null;
  wsClient: WebSocket | null = null;
  wsMessages: unknown[] = [];
  lastResponse: Response | null = null;
  lastResponseBody: unknown = null;
  lastBackendMessage: unknown = null;
  lastFileId: string | null = null;
  lastFileDownloadUrl: string | null = null;
  lastAdminResponse: Record<string, unknown> | null = null;
  lastUploadResponse: Response | null = null;
  lastUploadResponseBody: Record<string, unknown> | null = null;
  lastDownloadResponse: Response | null = null;
  lastDownloadResponseBody: string | null = null;
  lastTelegramSentMessage: Record<string, unknown> | null = null;
  pendingHeaders: Record<string, string> = {};

  constructor(options: IWorldOptions) {
    super(options);
  }

  async gatewayFetch(path: string, options: RequestInit = {}): Promise<Response> {
    if (!this.gateway) {
      throw new Error('Gateway not initialized. Call "Given a running gateway" first.');
    }
    const url = this.gateway.gatewayUrl + path;
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
      ...this.pendingHeaders,
      ...(options.headers as Record<string, string> | undefined ?? {}),
    };
    this.pendingHeaders = {};
    return fetch(url, { ...options, headers });
  }

  waitForWsMessage(timeoutMs: number): Promise<unknown> {
    if (!this.wsClient) {
      throw new Error('WebSocket client not initialized.');
    }
    if (this.wsMessages.length > 0) {
      return Promise.resolve(this.wsMessages.shift());
    }
    return new Promise((resolve, reject) => {
      const handler = (data: WebSocket.RawData) => {
        clearTimeout(timer);
        this.wsClient!.off('message', handler);
        try {
          resolve(JSON.parse(data.toString()));
        } catch (err) {
          reject(err);
        }
      };
      const timer = setTimeout(() => {
        this.wsClient!.off('message', handler);
        reject(new Error(`WS message not received within ${timeoutMs}ms`));
      }, timeoutMs);
      this.wsClient!.on('message', handler);
    });
  }
}

setWorldConstructor(TestWorld);

After(async function (this: TestWorld) {
  if (this.wsClient) {
    this.wsClient.close();
    this.wsClient = null;
    this.wsMessages = [];
  }
  if (this.gateway) {
    await this.gateway.stop();
    this.gateway = null;
  }
  if (this.backend) {
    await this.backend.stop();
    this.backend = null;
  }
  if (this.mockTelegramServer) {
    await this.mockTelegramServer.stop();
    this.mockTelegramServer = null;
  }
  if (this.mockFileServer) {
    await this.mockFileServer.stop();
    this.mockFileServer = null;
  }
});
