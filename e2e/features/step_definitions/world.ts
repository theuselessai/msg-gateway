import { World, setWorldConstructor, After, IWorldOptions } from '@cucumber/cucumber';
import { MockBackend } from '../../support/mock-backend';
import { TestGateway } from '../../support/test-gateway';
import WebSocket from 'ws';

export class TestWorld extends World {
  gateway: TestGateway | null = null;
  backend: MockBackend | null = null;
  wsClient: WebSocket | null = null;
  lastResponse: Response | null = null;
  lastResponseBody: unknown = null;
  lastBackendMessage: unknown = null;
  pendingHeaders: Record<string, string> = {};

  constructor(options: IWorldOptions) {
    super(options);
  }

  async gatewayFetch(path: string, options: RequestInit = {}): Promise<Response> {
    const url = this.gateway!.gatewayUrl + path;
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
      ...this.pendingHeaders,
      ...(options.headers as Record<string, string> | undefined ?? {}),
    };
    this.pendingHeaders = {};
    return fetch(url, { ...options, headers });
  }
}

setWorldConstructor(TestWorld);

After(async function (this: TestWorld) {
  if (this.wsClient) {
    this.wsClient.close();
    this.wsClient = null;
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
