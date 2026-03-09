import * as http from 'http';
import { URL } from 'url';

export class MockOpencodeServer {
  private server: http.Server;
  private _port: number = 0;
  private sessionRequests: unknown[] = [];
  private allMessages: unknown[] = [];
  private pendingMessages: unknown[] = [];
  private messageWaiters: Array<{ resolve: (msg: unknown) => void; reject: (err: Error) => void }> = [];

  constructor() {
    this.server = http.createServer((req, res) => this.handleRequest(req, res));
  }

  private handleRequest(req: http.IncomingMessage, res: http.ServerResponse): void {
    const url = new URL(req.url ?? '/', `http://localhost`);
    const pathname = url.pathname;

    let body = '';
    req.on('data', (chunk: Buffer) => { body += chunk.toString(); });
    req.on('error', () => {});
    req.on('end', () => {
      let parsed: unknown = null;
      if (body) {
        try { parsed = JSON.parse(body); } catch { /* ignore */ }
      }

      // GET /global/health
      if (req.method === 'GET' && pathname === '/global/health') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ healthy: true, version: 'test' }));
        return;
      }

      // POST /session
      if (req.method === 'POST' && pathname === '/session') {
        this.sessionRequests.push(parsed);
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ id: 'e2e-session-id' }));
        return;
      }

      // POST /session/:id/message
      const msgMatch = pathname.match(/^\/session\/([^/]+)\/message$/);
      if (req.method === 'POST' && msgMatch) {
        this.allMessages.push(parsed);
        if (this.messageWaiters.length > 0) {
          const waiter = this.messageWaiters.shift()!;
          waiter.resolve(parsed);
        } else {
          this.pendingMessages.push(parsed);
        }
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          info: { id: 'msg-e2e', role: 'assistant', finish: 'stop' },
          parts: [{ type: 'text', text: 'E2E mock AI response' }],
        }));
        return;
      }

      res.writeHead(404, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ error: 'not found' }));
    });
  }

  start(): Promise<number> {
    return new Promise((resolve, reject) => {
      const errorHandler = (err: Error) => reject(err);
      this.server.once('error', errorHandler);
      this.server.listen(0, '127.0.0.1', () => {
        this.server.removeListener('error', errorHandler);
        const addr = this.server.address();
        if (addr && typeof addr === 'object') {
          this._port = addr.port;
          resolve(this._port);
        } else {
          reject(new Error('Failed to get server address'));
        }
      });
    });
  }

  stop(): Promise<void> {
    for (const waiter of this.messageWaiters) {
      waiter.reject(new Error('MockOpencodeServer stopped'));
    }
    this.messageWaiters = [];
    return new Promise((resolve, reject) => {
      this.server.close((err) => {
        if (err) reject(err);
        else resolve();
      });
    });
  }

  waitForMessage(timeoutMs: number): Promise<unknown> {
    if (this.pendingMessages.length > 0) {
      return Promise.resolve(this.pendingMessages.shift());
    }
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        const idx = this.messageWaiters.findIndex((w) => w.resolve === resolve);
        if (idx !== -1) this.messageWaiters.splice(idx, 1);
        reject(new Error(`Timed out waiting for OpenCode message after ${timeoutMs}ms`));
      }, timeoutMs);

      this.messageWaiters.push({
        resolve: (msg) => {
          clearTimeout(timer);
          resolve(msg);
        },
        reject: (err) => {
          clearTimeout(timer);
          reject(err);
        },
      });
    });
  }

  getMessages(): unknown[] {
    return [...this.allMessages];
  }

  getSessionRequests(): unknown[] {
    return [...this.sessionRequests];
  }

  reset(): void {
    this.sessionRequests = [];
    this.allMessages = [];
    this.pendingMessages = [];
    for (const waiter of this.messageWaiters) {
      waiter.reject(new Error('MockOpencodeServer reset'));
    }
    this.messageWaiters = [];
  }

  get port(): number {
    return this._port;
  }

  get url(): string {
    return `http://127.0.0.1:${this._port}`;
  }
}
