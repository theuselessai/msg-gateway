import { World, setWorldConstructor, After, IWorldOptions } from '@cucumber/cucumber';
import { MockBackend } from '../../support/mock-backend';
import { TestGateway } from '../../support/test-gateway';
import WebSocket from 'ws';

export class TestWorld extends World {
  gateway: TestGateway | null = null;
  backend: MockBackend | null = null;
  wsClient: WebSocket | null = null;
  wsMessages: unknown[] = [];
  lastResponse: Response | null = null;
  lastResponseBody: unknown = null;
  lastBackendMessage: unknown = null;
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
      const timer = setTimeout(
        () => reject(new Error(`WS message not received within ${timeoutMs}ms`)),
        timeoutMs
      );
      const handler = (data: WebSocket.RawData) => {
        clearTimeout(timer);
        this.wsClient!.off('message', handler);
        try {
          resolve(JSON.parse(data.toString()));
        } catch (err) {
          reject(err);
        }
      };
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
});
