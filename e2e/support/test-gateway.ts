import * as net from 'net';
import * as fs from 'fs';
import * as os from 'os';
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
        server.close(() => reject(new Error('Failed to get free port')));
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

function buildConfig(
  gatewayPort: number,
  backend: MockBackend,
  fileCacheDir: string,
  extras?: {
    adaptersDir?: string;
    adapterPortRange?: [number, number];
    credentials?: Record<string, unknown>;
  }
): object {
  return {
    gateway: {
      listen: `127.0.0.1:${gatewayPort}`,
      admin_token: 'test_admin_token',
      default_backend: 'pipelit',
      adapters_dir: extras?.adaptersDir ?? '../adapters',
      adapter_port_range: extras?.adapterPortRange ?? [19000, 19100],
      file_cache: {
        directory: fileCacheDir,
        ttl_hours: 24,
        max_cache_size_mb: 100,
        cleanup_interval_minutes: 60,
        max_file_size_mb: 10,
        allowed_mime_types: [
          'image/jpeg',
          'image/png',
          'text/plain',
          'application/pdf',
          'application/octet-stream',
        ],
      },
    },
    backends: {
      pipelit: {
        protocol: 'pipelit',
        inbound_url: backend.inboundUrl,
        token: 'test_backend_token',
        active: true,
      },
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
      ...(extras?.credentials ?? {}),
    },
  };
}

async function pollHealth(url: string, maxMs: number): Promise<void> {
  const deadline = Date.now() + maxMs;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`${url}/health`);
      if (res.status === 200) return;
    } catch (_err) {}
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`Gateway at ${url} did not become healthy within ${maxMs}ms`);
}

export class TestGateway {
  private process: child_process.ChildProcess | null = null;
  private configPath: string | null = null;
  private fileCacheDir: string | null = null;
  private _gatewayPort: number = 0;
  private _adapterPortRange: [number, number] | null = null;

  readonly sendToken = 'test_send_token';
  readonly adminToken = 'test_admin_token';

  get gatewayUrl(): string {
    return `http://127.0.0.1:${this._gatewayPort}`;
  }

  async start(backend: MockBackend): Promise<void> {
    this._gatewayPort = await findFreePort();
    this.fileCacheDir = fs.mkdtempSync(path.join(os.tmpdir(), 'test-gateway-files-'));
    const config = buildConfig(this._gatewayPort, backend, this.fileCacheDir);

    this.configPath = path.join(
      os.tmpdir(),
      `test-gateway-config-${Date.now()}-${Math.random().toString(36).slice(2)}.json`
    );
    fs.writeFileSync(this.configPath, JSON.stringify(config, null, 2));

    const binary = findGatewayBinary();
    this.process = child_process.spawn(binary, [], {
      env: { ...process.env, GATEWAY_CONFIG: this.configPath },
      stdio: 'ignore',
    });

    this.process.on('error', (err) => {
      throw new Error(`Gateway process error: ${err.message}`);
    });

    await pollHealth(this.gatewayUrl, 10000);
  }

  async startWithOpencodeConfig(opencodePort: number): Promise<void> {
    this._gatewayPort = await findFreePort();
    this.fileCacheDir = fs.mkdtempSync(path.join(os.tmpdir(), 'test-gateway-files-'));
    const adapterPortBase = 20000 + Math.floor(Math.random() * 9000);
    this._adapterPortRange = [adapterPortBase, adapterPortBase + 100];
    const config = {
      gateway: {
        listen: `127.0.0.1:${this._gatewayPort}`,
        admin_token: 'test_admin_token',
        default_target: {
          protocol: 'opencode',
          base_url: `http://127.0.0.1:${opencodePort}`,
          token: 'testuser:testpass',
        },
        adapters_dir: '../adapters',
        adapter_port_range: this._adapterPortRange,
        file_cache: {
          directory: this.fileCacheDir,
          ttl_hours: 24,
          max_cache_size_mb: 100,
          cleanup_interval_minutes: 60,
          max_file_size_mb: 10,
          allowed_mime_types: [
            'image/jpeg',
            'image/png',
            'text/plain',
            'application/pdf',
            'application/octet-stream',
          ],
        },
      },
      auth: {
        send_token: 'test_send_token',
      },
      health_checks: {},
      credentials: {
        test_opencode: {
          adapter: 'generic',
          token: 'generic_token',
          active: true,
          emergency: false,
          route: { channel: 'test' },
          target: {
            protocol: 'opencode',
            base_url: `http://127.0.0.1:${opencodePort}`,
            token: 'testuser:testpass',
          },
          config: {
            model: { providerID: 'test', modelID: 'test-model' },
          },
        },
      },
    };

    this.configPath = path.join(
      os.tmpdir(),
      `test-gateway-config-${Date.now()}-${Math.random().toString(36).slice(2)}.json`
    );
    fs.writeFileSync(this.configPath, JSON.stringify(config, null, 2));

    const binary = findGatewayBinary();
    this.process = child_process.spawn(binary, [], {
      env: { ...process.env, GATEWAY_CONFIG: this.configPath },
      stdio: 'ignore',
    });

    this.process.on('error', (err) => {
      throw new Error(`Gateway process error: ${err.message}`);
    });

    await pollHealth(this.gatewayUrl, 10000);
  }

  async startWithTelegram(backend: MockBackend, telegramApiRoot: string): Promise<void> {
    this._gatewayPort = await findFreePort();
    this.fileCacheDir = fs.mkdtempSync(path.join(os.tmpdir(), 'test-gateway-files-'));
    const adapterPortBase = 20000 + Math.floor(Math.random() * 9000);
    this._adapterPortRange = [adapterPortBase, adapterPortBase + 100];
    const config = buildConfig(this._gatewayPort, backend, this.fileCacheDir, {
      adaptersDir: path.resolve(__dirname, '../../adapters'),
      adapterPortRange: this._adapterPortRange,
      credentials: {
        test_telegram: {
          adapter: 'telegram',
          token: 'test_bot_token',
          active: true,
          emergency: false,
          route: { channel: 'telegram' },
          config: { api_root: telegramApiRoot },
        },
      },
    });

    this.configPath = path.join(
      os.tmpdir(),
      `test-gateway-config-${Date.now()}-${Math.random().toString(36).slice(2)}.json`
    );
    fs.writeFileSync(this.configPath, JSON.stringify(config, null, 2));

    const binary = findGatewayBinary();
    this.process = child_process.spawn(binary, [], {
      env: { ...process.env, GATEWAY_CONFIG: this.configPath },
      stdio: 'ignore',
    });

    this.process.on('error', (err) => {
      throw new Error(`Gateway process error: ${err.message}`);
    });

    await pollHealth(this.gatewayUrl, 20000);
    // Allow adapter polling loop to start after health check passes
    await new Promise((r) => setTimeout(r, 1000));
  }

  async stop(): Promise<void> {
    if (this.process) {
      const pid = this.process.pid;
      await new Promise<void>((resolve) => {
        const timer = setTimeout(() => {
          this.process?.kill('SIGKILL');
          resolve();
        }, 3000);
        this.process!.on('exit', () => {
          clearTimeout(timer);
          resolve();
        });
        this.process!.kill('SIGTERM');
      });
      this.process = null;

      // Kill any orphaned child processes (e.g. adapter subprocesses)
      if (pid) {
        try {
          child_process.execSync(`pkill -P ${pid} 2>/dev/null`, { stdio: 'ignore' });
        } catch {}
      }
    }

    // Kill any processes still listening on the adapter port range
    if (this._adapterPortRange) {
      for (let port = this._adapterPortRange[0]; port <= Math.min(this._adapterPortRange[0] + 10, this._adapterPortRange[1]); port++) {
        try {
          child_process.execSync(`fuser -k ${port}/tcp 2>/dev/null`, { stdio: 'ignore' });
        } catch {}
      }
      this._adapterPortRange = null;
    }

    if (this.configPath && fs.existsSync(this.configPath)) {
      fs.unlinkSync(this.configPath);
      this.configPath = null;
    }
    if (this.fileCacheDir && fs.existsSync(this.fileCacheDir)) {
      fs.rmSync(this.fileCacheDir, { recursive: true, force: true });
      this.fileCacheDir = null;
    }
  }
}
