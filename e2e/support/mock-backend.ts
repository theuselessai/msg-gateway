import * as http from 'http';

export class MockBackend {
  private server: http.Server;
  private messages: unknown[] = [];
  private waiters: Array<{ resolve: (msg: unknown) => void; reject: (err: Error) => void }> = [];
  private _port: number = 0;

  constructor() {
    this.server = http.createServer((req, res) => {
      if (req.method === 'POST' && req.url === '/inbound') {
        let body = '';
        req.on('data', (chunk: Buffer) => {
          body += chunk.toString();
        });
        req.on('error', () => {});
        req.on('end', () => {
          let parsed: unknown;
          try {
            parsed = JSON.parse(body);
          } catch {
            res.writeHead(400, { 'Content-Type': 'application/json' });
            res.end(JSON.stringify({ error: 'invalid json' }));
            return;
          }
          this.messages.push(parsed);
          if (this.waiters.length > 0) {
            const waiter = this.waiters.shift()!;
            waiter.resolve(parsed);
          }
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ status: 'ok' }));
        });
      } else {
        res.writeHead(404, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: 'not found' }));
      }
    });
  }

  start(): Promise<void> {
    return new Promise((resolve, reject) => {
      const errorHandler = (err: Error) => reject(err);
      this.server.once('error', errorHandler);
      this.server.listen(0, '127.0.0.1', () => {
        this.server.removeListener('error', errorHandler);
        const addr = this.server.address();
        if (addr && typeof addr === 'object') {
          this._port = addr.port;
          resolve();
        } else {
          reject(new Error('Failed to get server address'));
        }
      });
    });
  }

  stop(): Promise<void> {
    return new Promise((resolve, reject) => {
      for (const waiter of this.waiters) {
        waiter.reject(new Error('MockBackend stopped'));
      }
      this.waiters = [];
      this.server.close((err) => {
        if (err) reject(err);
        else resolve();
      });
    });
  }

  get port(): number {
    return this._port;
  }

  get inboundUrl(): string {
    return `http://127.0.0.1:${this._port}/inbound`;
  }

  waitForMessage(timeoutMs: number): Promise<unknown> {
    if (this.messages.length > 0) {
      return Promise.resolve(this.messages.shift());
    }
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        const idx = this.waiters.findIndex((w) => w.resolve === resolve);
        if (idx !== -1) this.waiters.splice(idx, 1);
        reject(new Error(`Timed out waiting for backend message after ${timeoutMs}ms`));
      }, timeoutMs);

      this.waiters.push({
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
    return [...this.messages];
  }

  reset(): void {
    this.messages = [];
    for (const waiter of this.waiters) {
      waiter.reject(new Error('MockBackend reset'));
    }
    this.waiters = [];
  }
}
