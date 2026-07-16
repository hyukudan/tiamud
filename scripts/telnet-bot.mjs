import net from 'node:net';
import { performance } from 'node:perf_hooks';

const IAC = 255;
const DONT = 254;
const DO = 253;
const WONT = 252;
const WILL = 251;
const SB = 250;
const SE = 240;
const OPT_ECHO = 1;
const OPT_SUPPRESS_GO_AHEAD = 3;
const OPT_EOR = 25;
const MAX_SUBNEGOTIATION_BYTES = 16 * 1024;
const MAX_TEXT_BUFFER_BYTES = 256 * 1024;
const REQUEST_TIMEOUT_MS = 10_000;

const ACCEPTED_SERVER_OPTIONS = new Set([OPT_ECHO, OPT_SUPPRESS_GO_AHEAD, OPT_EOR]);

export class TelnetByteParser {
  #state = 'data';
  #negotiationCommand;
  #subnegotiationOption;
  #subnegotiationBytes = 0;
  #subnegotiationOverflowed = false;
  #replyToNegotiation;

  constructor({ replyToNegotiation = true } = {}) {
    this.#replyToNegotiation = replyToNegotiation;
  }

  push(chunk) {
    const data = [];
    const response = [];
    const events = [];
    for (const byte of chunk) {
      switch (this.#state) {
        case 'data':
          if (byte === IAC) this.#state = 'iac';
          else data.push(byte);
          break;
        case 'iac':
          if (byte === IAC) {
            data.push(IAC);
            this.#state = 'data';
          } else if ([WILL, WONT, DO, DONT].includes(byte)) {
            this.#negotiationCommand = byte;
            this.#state = 'negotiation';
          } else if (byte === SB) {
            this.#state = 'subnegotiation-option';
          } else {
            events.push({ kind: 'command', command: byte });
            this.#state = 'data';
          }
          break;
        case 'negotiation': {
          const command = this.#negotiationCommand;
          events.push({ kind: 'negotiation', command, option: byte });
          if (this.#replyToNegotiation) response.push(...negotiationReply(command, byte));
          this.#state = 'data';
          break;
        }
        case 'subnegotiation-option':
          if (byte === IAC) {
            events.push({ kind: 'malformed-subnegotiation' });
            this.#state = 'iac';
          } else {
            this.#subnegotiationOption = byte;
            this.#subnegotiationBytes = 0;
            this.#subnegotiationOverflowed = false;
            this.#state = 'subnegotiation';
          }
          break;
        case 'subnegotiation':
          if (byte === IAC) this.#state = 'subnegotiation-iac';
          else this.#pushSubnegotiationByte();
          break;
        case 'subnegotiation-iac':
          if (byte === SE) {
            events.push(
              this.#subnegotiationOverflowed
                ? { kind: 'subnegotiation-too-long', option: this.#subnegotiationOption }
                : {
                    kind: 'subnegotiation',
                    option: this.#subnegotiationOption,
                    bytes: this.#subnegotiationBytes,
                  },
            );
            this.#state = 'data';
          } else {
            if (byte === IAC) this.#pushSubnegotiationByte();
            this.#state = 'subnegotiation';
          }
          break;
        default:
          throw new Error(`Unknown Telnet parser state ${this.#state}.`);
      }
    }
    return {
      data: Buffer.from(data),
      response: Buffer.from(response),
      events,
    };
  }

  #pushSubnegotiationByte() {
    if (this.#subnegotiationOverflowed) return;
    if (this.#subnegotiationBytes === MAX_SUBNEGOTIATION_BYTES) {
      this.#subnegotiationBytes = 0;
      this.#subnegotiationOverflowed = true;
      return;
    }
    this.#subnegotiationBytes += 1;
  }
}

export function parseTelnetEndpoint(value) {
  const url = new URL(value.includes('://') ? value : `telnet://${value}`);
  if (
    url.protocol !== 'telnet:' ||
    url.username ||
    url.password ||
    !['', '/'].includes(url.pathname) ||
    url.search ||
    url.hash
  ) {
    throw new Error('--telnet must be a plaintext host:port endpoint without credentials or a path.');
  }
  const port = Number(url.port);
  if (!url.hostname || !Number.isSafeInteger(port) || port < 1 || port > 65_535) {
    throw new Error('--telnet must include a valid host and port.');
  }
  return { host: url.hostname.replace(/^\[|\]$/g, ''), port, display: `${url.hostname}:${port}` };
}

export async function provisionTelnetBot(endpoint, runId, index, recordLatency = () => {}) {
  return new TiamudTelnetBot({ endpoint, runId, index, recordLatency });
}

export class TiamudTelnetBot {
  #endpoint;
  #index;
  #account;
  #password;
  #character;
  #recordLatency;
  #connection;
  #roundCounter = 0;
  #paceMs = 0;
  #lastActionAt = 0;
  #location = 'square';

  constructor({ endpoint, runId, index, recordLatency = () => {} }) {
    this.#endpoint = endpoint;
    this.#index = index;
    this.#account = `Telnet-${runId}-${index}`.slice(0, 32);
    this.#password = `telnet-load-${runId}`;
    this.#character = `Telnet Explorer ${index}`;
    this.#recordLatency = recordLatency;
  }

  get transport() {
    return 'telnet';
  }

  set paceMs(value) {
    this.#paceMs = value;
  }

  async bootstrap() {
    await this.#connectAndAuthenticate('register');
    await this.#command('north', 'The North Gate', 'command.north');
    this.#location = 'gate';
    await this.#command('look', 'The North Gate', 'command.look.gate');
    await this.#disconnectCleanly();
    await this.#connectAndAuthenticate('login');
    await this.#command('look', 'The North Gate', 'command.look.reconnect');
  }

  async runRound() {
    if (!this.#connection) throw new Error(`Telnet bot ${this.#index} is not connected.`);
    this.#roundCounter += 1;
    await this.#command('south', 'Lantern Square', 'command.south');
    this.#location = 'square';
    if (this.#roundCounter % 8 === 1) {
      await this.#command(
        `say Telnet soak pulse ${this.#index}-${this.#roundCounter}`,
        'You say',
        'command.say',
      );
    }
    await this.#command('look', 'Lantern Square', 'command.look.square');
    await this.#command('north', 'The North Gate', 'command.north');
    this.#location = 'gate';
    await this.#command('look', 'The North Gate', 'command.look.gate');
    if (this.#roundCounter % 16 === 0) {
      await this.#disconnectCleanly();
      await this.#connectAndAuthenticate('login', 'periodic-reconnect');
    }
  }

  async close() {
    await this.#disconnectCleanly();
  }

  async #connectAndAuthenticate(mode, metric = `auth.${mode}`) {
    const connection = new TelnetTextConnection(this.#endpoint);
    const started = performance.now();
    await connection.connect();
    this.#recordLatency('connect', performance.now() - started);
    this.#connection = connection;
    try {
      await connection.waitFor('Access [login/register]: ', 'access prompt');
      connection.sendLine(mode);
      await connection.waitFor('Account: ', 'account prompt');
      connection.sendLine(this.#account);
      await connection.waitFor('Password: ', 'password prompt');
      connection.sendLine(this.#password);
      if (mode === 'register') {
        await connection.waitFor('Name your first character: ', 'character creation prompt');
        connection.sendLine(this.#character);
      } else {
        await connection.waitFor(
          'Choose a number, or type a new character name: ',
          'character selection prompt',
        );
        connection.sendLine('1');
      }
      const transcript = mode === 'register'
        ? await finishDefaultCharacterCreation(connection)
        : await connection.waitFor('> ', 'game prompt');
      const expected = this.#location === 'gate' ? 'The North Gate' : 'Lantern Square';
      if (!transcript.includes(expected)) {
        throw new Error(`Telnet bot ${this.#index} entered an unexpected scene.`);
      }
      this.#recordLatency(metric, performance.now() - started);
    } catch (error) {
      await connection.close().catch(() => {});
      this.#connection = undefined;
      throw error;
    }
  }

  async #command(command, expectedText, metric) {
    await this.#pace();
    const started = performance.now();
    // A busy MUD can render an unsolicited scene and prompt between commands.
    // Do not let a stale title/prompt satisfy the command we are about to send.
    // Clearing is synchronous with sendLine, so data received afterwards is
    // causally later than this harness-side boundary.
    this.#connection.discardBufferedText();
    this.#connection.sendLine(command);
    // Telnet sessions can receive asynchronous scene/chat updates, each with
    // its own prompt. Wait for command-specific evidence before accepting the
    // following prompt so another player's update cannot complete the action.
    await this.#connection.waitFor(expectedText, `result of ${command}`);
    await this.#connection.waitFor('> ', `prompt after ${command}`);
    this.#recordLatency(metric, performance.now() - started);
    this.#lastActionAt = performance.now();
  }

  async #pace() {
    const remaining = this.#lastActionAt + this.#paceMs - performance.now();
    if (remaining > 0) await delay(remaining);
  }

  async #disconnectCleanly() {
    const connection = this.#connection;
    if (!connection) return;
    this.#connection = undefined;
    try {
      connection.sendLine('quit');
      await connection.waitFor('Goodbye.', 'clean Telnet shutdown', 2_000);
    } finally {
      await connection.close();
    }
  }
}

class TelnetTextConnection {
  #endpoint;
  #socket;
  #parser = new TelnetByteParser();
  #decoder = new TextDecoder('utf-8', { fatal: false });
  #text = '';
  #waiter;
  #terminalError;

  constructor(endpoint) {
    this.#endpoint = endpoint;
  }

  async connect() {
    const socket = net.createConnection(this.#endpoint);
    this.#socket = socket;
    socket.setNoDelay(true);
    socket.setKeepAlive(true, 5_000);
    socket.on('data', (chunk) => this.#receive(chunk));
    socket.on('error', (error) => this.#terminate(error));
    socket.on('close', () => this.#terminate(new Error('Telnet connection closed.')));
    await Promise.race([
      new Promise((resolve, reject) => {
        socket.once('connect', resolve);
        socket.once('error', reject);
      }),
      timeout(REQUEST_TIMEOUT_MS, 'Telnet connect timed out.'),
    ]);
  }

  sendLine(value) {
    if (!this.#socket || this.#socket.destroyed) throw new Error('Telnet socket is closed.');
    this.#socket.write(`${value}\r\n`);
  }

  discardBufferedText() {
    this.#text = '';
  }

  async waitFor(marker, label, timeoutMs = REQUEST_TIMEOUT_MS) {
    const result = await this.waitForAny([marker], label, timeoutMs);
    return result.transcript;
  }

  async waitForAny(markers, label, timeoutMs = REQUEST_TIMEOUT_MS) {
    const deadline = performance.now() + timeoutMs;
    for (;;) {
      const found = markers
        .map((marker) => ({ marker, index: this.#text.indexOf(marker) }))
        .filter((candidate) => candidate.index >= 0)
        .sort((left, right) => left.index - right.index)[0];
      if (found) {
        const end = found.index + found.marker.length;
        const transcript = this.#text.slice(0, end);
        this.#text = this.#text.slice(end);
        return { marker: found.marker, transcript };
      }
      if (this.#terminalError) throw this.#terminalError;
      const remaining = deadline - performance.now();
      if (remaining <= 0) throw new Error(`Timed out waiting for ${label}.`);
      await new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
          this.#waiter = undefined;
          reject(new Error(`Timed out waiting for ${label}.`));
        }, remaining);
        this.#waiter = () => {
          clearTimeout(timer);
          this.#waiter = undefined;
          resolve();
        };
      });
    }
  }

  async close() {
    const socket = this.#socket;
    if (!socket || socket.destroyed) return;
    await Promise.race([
      new Promise((resolve) => {
        socket.once('close', resolve);
        socket.end();
      }),
      delay(1_000).then(() => socket.destroy()),
    ]);
  }

  #receive(chunk) {
    const parsed = this.#parser.push(chunk);
    if (parsed.response.length > 0 && this.#socket && !this.#socket.destroyed) {
      this.#socket.write(parsed.response);
    }
    if (parsed.data.length > 0) {
      this.#text += this.#decoder.decode(parsed.data, { stream: true });
      if (Buffer.byteLength(this.#text) > MAX_TEXT_BUFFER_BYTES) {
        this.#terminate(new Error('Telnet text buffer exceeded its safety bound.'));
      }
    }
    this.#wake();
  }

  #terminate(error) {
    this.#terminalError ??= error instanceof Error ? error : new Error(String(error));
    this.#wake();
  }

  #wake() {
    this.#waiter?.();
  }
}

async function finishDefaultCharacterCreation(connection) {
  const gamePrompt = '> ';
  const selectionPrompts = [
    'Choose a number or lineage id (type default): ',
    'Choose a number or background id (type default or none): ',
    'Choose a number or base class id (type default): ',
  ];
  for (let step = 0; step <= selectionPrompts.length; step += 1) {
    const result = await connection.waitForAny(
      [gamePrompt, ...selectionPrompts],
      'character progression or game prompt',
    );
    if (result.marker === gamePrompt) return result.transcript;
    connection.sendLine('default');
  }
  throw new Error('Telnet character creation exceeded its bounded progression prompts.');
}

function negotiationReply(command, option) {
  if (command === WILL) {
    return [IAC, ACCEPTED_SERVER_OPTIONS.has(option) ? DO : DONT, option];
  }
  if (command === DO) return [IAC, WONT, option];
  return [];
}

function timeout(milliseconds, message) {
  return new Promise((_, reject) => {
    const timer = setTimeout(() => reject(new Error(message)), milliseconds);
    timer.unref();
  });
}

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

export const TELNET_BYTES = Object.freeze({ IAC, DONT, DO, WONT, WILL, SB, SE });
