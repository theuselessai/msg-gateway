import * as net from 'net';
import * as fs from 'fs';
import * as path from 'path';
import * as child_process from 'child_process';
import { MockBackend } from './mock-backend';

function findFreePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.listen(0, '127.0.0.1', () => {
      const addr = server.address();
      if (addr && typeof addr === 'object') {
        const port = addr.port;
        server.close(() => resolve(port));
      } else {
        server.close();
        reject(new Error('Failed to get free port'));
      }
    });
    server.on('error', reject);
  });
}

function findGatewayBinary(): string {
  const dir = path.resolve(__dirname, '../../');
  const release = path.join(dir, 'target/release/msg-gateway');
  const debug = path.join(dir, 'target/debug/msg-gateway');
  if (fs.existsSync(release)) return release;
  if (fs.existsSync(debug)) return debug;
  throw new Error(`Gateway binary not found at ${release} or ${debug}. Run 'cargo build' first.`);
}

function buildConfig(gatewayPort: number, backend: MockBackend): object {
  return {
    gateway: {
      listen: `127.0.0.1:${gatewayPort}`,
      admin_token: 'test_admin_token',
      default_target: {
        protocol: 'pipelit',
        inbound_url: backend.inboundUrl,
        token: 'test_backend_token',
      },
      adapters_dir: '../adapters',
      adapter_port_range: [19000, 19100],
    },
    auth: {
      send_token: 'test_send_token',
    },
    health_checks: {},
    credentials: {
      test_generic: {
        adapter: 'generic',
        token: 'generic_token',
        active: true,
        emergency: false,
        route: { channel: 'test' },
      },
    },
  };
}

async function pollHealth(url: string, maxMs: number): Promise<void> {
  const deadline = Date.now() + maxMs;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`${url}/health`);
      if (res.status === 200) return;
    } catch (_err) {
      void _err;
    }
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`Gateway at ${url} did not become healthy within ${maxMs}ms`);
}

export class TestGateway {
  private process: child_process.ChildProcess | null = null;
  private configPath: string | null = null;
  private _gatewayPort: number = 0;

  readonly sendToken = 'test_send_token';
  readonly adminToken = 'test_admin_token';

  get gatewayUrl(): string {
    return `http://127.0.0.1:${this._gatewayPort}`;
  }

  async start(backend: MockBackend): Promise<void> {
    this._gatewayPort = await findFreePort();
    const config = buildConfig(this._gatewayPort, backend);

    this.configPath = path.join(
      require('os').tmpdir(),
      `test-gateway-config-${Date.now()}-${Math.random().toString(36).slice(2)}.json`
    );
    fs.writeFileSync(this.configPath, JSON.stringify(config, null, 2));

    const binary = findGatewayBinary();
    this.process = child_process.spawn(binary, [], {
      env: { ...process.env, GATEWAY_CONFIG: this.configPath },
      stdio: ['ignore', 'pipe', 'pipe'],
    });

    this.process.stdout?.on('data', () => {});
    this.process.stderr?.on('data', () => {});

    this.process.on('error', (err) => {
      throw new Error(`Gateway process error: ${err.message}`);
    });

    await pollHealth(this.gatewayUrl, 10000);
  }

  async stop(): Promise<void> {
    if (this.process) {
      this.process.kill('SIGTERM');
      await new Promise<void>((resolve) => {
        const timer = setTimeout(() => {
          this.process?.kill('SIGKILL');
          resolve();
        }, 3000);
        this.process!.on('exit', () => {
          clearTimeout(timer);
          resolve();
        });
      });
      this.process = null;
    }
    if (this.configPath && fs.existsSync(this.configPath)) {
      fs.unlinkSync(this.configPath);
      this.configPath = null;
    }
  }
}
