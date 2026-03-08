import * as http from 'http';

export class MockFileServer {
  private server: http.Server;
  private _port: number = 0;

  constructor() {
    this.server = http.createServer((req, res) => {
      if (req.method === 'GET' && req.url === '/test.txt') {
        const content = 'hello world';
        res.writeHead(200, { 'Content-Type': 'text/plain', 'Content-Length': String(Buffer.byteLength(content)) });
        res.end(content);
      } else {
        res.writeHead(404);
        res.end('not found');
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
      this.server.close(err => err ? reject(err) : resolve());
    });
  }

  get port(): number { return this._port; }
  get url(): string { return `http://127.0.0.1:${this._port}`; }
  get fileUrl(): string { return `${this.url}/test.txt`; }
}
