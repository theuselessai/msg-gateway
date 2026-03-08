import * as http from 'http';
import { URL } from 'url';

interface TelegramUpdate {
  update_id: number;
  message: {
    message_id: number;
    from: { id: number; first_name: string; username?: string; is_bot: false };
    chat: { id: number; type: string };
    date: number;
    text?: string;
  };
}

interface SentMessage {
  chat_id: string;
  text?: string;
  caption?: string;
  reply_to_message_id?: string;
  type: 'text' | 'photo' | 'document';
}

export class MockTelegramServer {
  private server: http.Server;
  private _port: number = 0;
  private updateQueue: TelegramUpdate[] = [];
  private nextUpdateId: number = 1;
  private nextMessageId: number = 1;
  private sentMessages: SentMessage[] = [];
  private sentWaiters: Array<{ resolve: (msg: SentMessage) => void; reject: (err: Error) => void }> = [];
  private pendingPolls: Array<{ res: http.ServerResponse; offset: number; timer: ReturnType<typeof setTimeout> }> = [];
  private _requestLog: string[] = [];

  constructor() {
    this.server = http.createServer((req, res) => this.handleRequest(req, res));
  }

  private handleRequest(req: http.IncomingMessage, res: http.ServerResponse): void {
    const url = new URL(req.url ?? '/', `http://localhost`);
    const parts = url.pathname.split('/').filter(Boolean);
    // parts[0] = 'bot{token}', parts[1] = method
    const method = parts[1] ?? '';

    let body = '';
    req.on('data', (chunk: Buffer) => { body += chunk.toString(); });
    req.on('end', () => {
      let parsedBody: Record<string, unknown> = {};
      try { parsedBody = body ? JSON.parse(body) : {}; } catch { /* ignore */ }

      const queryOffset = url.searchParams.get('offset');
      const queryTimeout = url.searchParams.get('timeout');
      const offset = queryOffset ? parseInt(queryOffset) : (typeof parsedBody.offset === 'number' ? parsedBody.offset : 0);
      const timeout = queryTimeout ? parseInt(queryTimeout) : (typeof parsedBody.timeout === 'number' ? parsedBody.timeout : 0);

      this._requestLog.push(method);
      this.dispatch(method, offset, timeout, parsedBody, res);
    });
    req.on('error', () => {});
  }

  private dispatch(method: string, offset: number, timeout: number, body: Record<string, unknown>, res: http.ServerResponse): void {
    switch (method) {
      case 'getMe':
        this.respond(res, {
          id: 123456, is_bot: true, first_name: 'TestBot', username: 'testbot',
          can_join_groups: true, can_read_all_group_messages: false, supports_inline_queries: false,
        });
        break;
      case 'getUpdates':
        this.handleGetUpdates(offset, timeout, res);
        break;
      case 'sendMessage':
        this.handleSendMessage(body, 'text', res);
        break;
      case 'sendPhoto':
        this.handleSendMessage(body, 'photo', res);
        break;
      case 'sendDocument':
        this.handleSendMessage(body, 'document', res);
        break;
      case 'deleteWebhook':
      case 'setMyCommands':
      case 'getMyCommands':
      case 'logOut':
      case 'close':
        this.respond(res, true);
        break;
      default:
        this.respond(res, true);
    }
  }

  private handleGetUpdates(offset: number, timeout: number, res: http.ServerResponse): void {
    const relevant = this.updateQueue.filter(u => u.update_id >= offset);
    if (relevant.length > 0) {
      this.respond(res, relevant);
      return;
    }
    const holdMs = Math.min((timeout || 0) * 1000, 1000);
    if (holdMs <= 0) {
      this.respond(res, []);
      return;
    }
    const timer = setTimeout(() => {
      const idx = this.pendingPolls.findIndex(p => p.res === res);
      if (idx !== -1) this.pendingPolls.splice(idx, 1);
      this.respond(res, []);
    }, holdMs);
    this.pendingPolls.push({ res, offset, timer });
  }

  private handleSendMessage(body: Record<string, unknown>, type: SentMessage['type'], res: http.ServerResponse): void {
    const msgId = this.nextMessageId++;
    const msg: SentMessage = {
      chat_id: String(body.chat_id ?? ''),
      text: typeof body.text === 'string' ? body.text : undefined,
      caption: typeof body.caption === 'string' ? body.caption : undefined,
      type,
    };
    if (body.reply_parameters && typeof body.reply_parameters === 'object') {
      msg.reply_to_message_id = String((body.reply_parameters as Record<string, unknown>).message_id ?? '');
    }
    this.sentMessages.push(msg);
    if (this.sentWaiters.length > 0) {
      const waiter = this.sentWaiters.shift()!;
      waiter.resolve(msg);
    }
    this.respond(res, {
      message_id: msgId,
      from: { id: 123456, is_bot: true, first_name: 'TestBot', username: 'testbot' },
      chat: { id: parseInt(msg.chat_id, 10) || 0, type: 'private' },
      date: Math.floor(Date.now() / 1000),
      text: msg.text,
    });
  }

  private respond(res: http.ServerResponse, result: unknown): void {
    const json = JSON.stringify({ ok: true, result });
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(json);
  }

  injectTextMessage(chatId: number, text: string, from: { id: number; first_name: string; username?: string }): void {
    const update: TelegramUpdate = {
      update_id: this.nextUpdateId++,
      message: {
        message_id: this.nextMessageId++,
        from: { ...from, is_bot: false },
        chat: { id: chatId, type: 'private' },
        date: Math.floor(Date.now() / 1000),
        text,
      },
    };
    this.updateQueue.push(update);
    if (this.pendingPolls.length > 0) {
      const poll = this.pendingPolls.shift()!;
      clearTimeout(poll.timer);
      const relevant = this.updateQueue.filter(u => u.update_id >= poll.offset);
      this.respond(poll.res, relevant);
    }
  }

  waitForSentMessage(timeoutMs: number): Promise<SentMessage> {
    if (this.sentMessages.length > 0) {
      return Promise.resolve(this.sentMessages.shift()!);
    }
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        const idx = this.sentWaiters.findIndex(w => w.resolve === resolve);
        if (idx !== -1) this.sentWaiters.splice(idx, 1);
        reject(new Error(`No Telegram sendMessage received within ${timeoutMs}ms`));
      }, timeoutMs);
      this.sentWaiters.push({
        resolve: (msg) => { clearTimeout(timer); resolve(msg); },
        reject: (err) => { clearTimeout(timer); reject(err); },
      });
    });
  }

  getSentMessages(): SentMessage[] { return [...this.sentMessages]; }

  reset(): void {
    this.updateQueue = [];
    this.sentMessages = [];
    for (const w of this.sentWaiters) w.reject(new Error('MockTelegramServer reset'));
    this.sentWaiters = [];
    for (const p of this.pendingPolls) { clearTimeout(p.timer); this.respond(p.res, []); }
    this.pendingPolls = [];
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
    for (const p of this.pendingPolls) { clearTimeout(p.timer); try { p.res.end(''); } catch {} }
    this.pendingPolls = [];
    for (const w of this.sentWaiters) w.reject(new Error('MockTelegramServer stopped'));
    this.sentWaiters = [];
    return new Promise((resolve, reject) => {
      this.server.close(err => err ? reject(err) : resolve());
    });
  }

  get port(): number { return this._port; }
  get url(): string { return `http://127.0.0.1:${this._port}`; }
  get requestLog(): string[] { return [...this._requestLog]; }
}
