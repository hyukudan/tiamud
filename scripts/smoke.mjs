import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtemp, rm } from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';

const root = path.resolve(import.meta.dirname, '..');
const assetHash = '8ade88f0df1adb294333e72d3fa2d0863543dc0263ebe3038208f3c9e415924c';
const assetManifestPath = `/assets/${assetHash}/manifest.json`;
const temporary = await mkdtemp(path.join(os.tmpdir(), 'tiamud-smoke-'));
const httpPort = await reservePort();
const telnetPort = await reservePort();
let output = '';

const server = spawn('cargo', ['run', '--locked', '-p', 'tiamud-server'], {
  cwd: root,
  env: {
    ...process.env,
    TIAMUD_DB_PATH: path.join(temporary, 'smoke.db'),
    TIAMUD_HTTP_ADDR: `127.0.0.1:${httpPort}`,
    TIAMUD_TELNET_ADDR: `127.0.0.1:${telnetPort}`,
    TIAMUD_TELNET_MODE: 'plaintext',
  },
  stdio: ['ignore', 'pipe', 'pipe'],
});

server.stdout.on('data', (chunk) => (output += chunk));
server.stderr.on('data', (chunk) => (output += chunk));

try {
  await waitUntilReady(httpPort, server);
  await Promise.all([
    verifyBrowser(httpPort),
    verifyTelnet(telnetPort),
    verifyWebBundle(httpPort),
    verifyAtmosphericAsset(httpPort),
  ]);
  console.log(
    'Smoke test passed: auth, reconnect, inventory, WorldPack, SQLite, assets, tiamud.v1, and Telnet share one server.',
  );
} catch (error) {
  console.error(output);
  throw error;
} finally {
  server.kill('SIGTERM');
  await Promise.race([onceExit(server), delay(2_000)]);
  if (server.exitCode === null) {
    server.kill('SIGKILL');
  }
  await rm(temporary, { recursive: true, force: true });
}

async function reservePort() {
  const socket = net.createServer();
  await new Promise((resolve, reject) => {
    socket.once('error', reject);
    socket.listen(0, '127.0.0.1', resolve);
  });
  const address = socket.address();
  const port = typeof address === 'object' && address ? address.port : undefined;
  await new Promise((resolve) => socket.close(resolve));
  if (!port) throw new Error('Could not reserve a local port.');
  return port;
}

async function waitUntilReady(port, child) {
  for (let attempt = 0; attempt < 300; attempt += 1) {
    if (child.exitCode !== null) {
      throw new Error(`Server exited before becoming ready (${child.exitCode}).`);
    }
    try {
      const response = await fetch(`http://127.0.0.1:${port}/health/ready`);
      if (response.ok) {
        const status = await response.json();
        if (
          status.status === 'ready' &&
          status.world === true &&
          status.storage?.backend === 'sqlite' &&
          status.storage?.ready === true
        ) return;
      }
    } catch {
      // The listener may still be starting.
    }
    await delay(50);
  }
  throw new Error('Server did not become ready within fifteen seconds.');
}

async function verifyBrowser(port) {
  const base = `http://127.0.0.1:${port}`;
  const account = `Browser-${Date.now()}`;
  const auth = await jsonRequest(`${base}/api/auth/register`, {
    method: 'POST',
    body: JSON.stringify({ name: account, password: 'smoke-password' }),
  });
  const created = await jsonRequest(`${base}/api/characters`, {
    method: 'POST',
    headers: { authorization: `Bearer ${auth.token}` },
    body: JSON.stringify({ name: 'Web Explorer' }),
  });
  const credentials = { token: auth.token, characterId: created.character.id };
  await browserJourney(port, credentials, false);
  await delay(100);
  await browserJourney(port, credentials, true);
}

function browserJourney(port, credentials, reconnecting) {
  return new Promise((resolve, reject) => {
    const websocket = new WebSocket(`ws://127.0.0.1:${port}/ws`, 'tiamud.v1');
    let state = reconnecting ? 'reconnect' : 'initial';
    const timer = setTimeout(() => reject(new Error('WebSocket smoke test timed out.')), 5_000);

    websocket.addEventListener('open', () => {
      websocket.send(
        JSON.stringify({
          v: 1,
          type: 'client.hello',
          data: {
            client: 'web',
            capabilities: {
              images: ['avif', 'webp', 'jpeg'],
              reducedMotion: true,
              audio: false,
            },
            auth: credentials,
          },
        }),
      );
    });
    websocket.addEventListener('message', (event) => {
      const message = JSON.parse(event.data);
      if (message.type === 'action.result' && state === 'inventory') {
        if (!message.data.ok || !message.data.message?.toLowerCase().includes('lantern')) {
          reject(new Error(`Inventory was not restored after reconnect: ${message.data.message}`));
          return;
        }
        clearTimeout(timer);
        websocket.close();
        resolve();
        return;
      }
      if (message.type !== 'scene.snapshot') return;
      if (state === 'initial') {
        if (message.data.title !== 'Lantern Square' || message.data.viewId.includes('demo:')) {
          reject(new Error(`Unexpected or non-opaque initial browser scene: ${JSON.stringify(message.data)}`));
          return;
        }
        if (message.data.image?.manifest !== assetManifestPath) {
          reject(new Error(`Unexpected scene asset manifest: ${message.data.image?.manifest}`));
          return;
        }
        if (!message.data.items?.some(
          (item) => item.label === 'blue-glass lantern' && !item.id.includes('tiamud:'),
        )) {
          reject(new Error('Initial browser scene is missing the blue lantern.'));
          return;
        }
        state = 'taken';
        websocket.send(
          JSON.stringify({
            v: 1,
            type: 'command.submit',
            data: { requestId: 'smoke-browser-take', command: 'take lantern' },
          }),
        );
      } else if (state === 'taken' && message.data.title === 'Lantern Square') {
        state = 'moving';
        websocket.send(
          JSON.stringify({
            v: 1,
            type: 'command.submit',
            data: { requestId: 'smoke-browser-north', command: 'north' },
          }),
        );
      } else if (state === 'moving' && message.data.title === 'The North Gate') {
        clearTimeout(timer);
        websocket.close();
        resolve();
      } else if (state === 'reconnect') {
        if (message.data.title !== 'The North Gate' || message.data.viewId.includes('demo:')) {
          reject(new Error(`Reconnect resumed in an unexpected scene: ${JSON.stringify(message.data)}.`));
          return;
        }
        state = 'inventory';
        websocket.send(
          JSON.stringify({
            v: 1,
            type: 'command.submit',
            data: { requestId: 'smoke-browser-inventory', command: 'inventory' },
          }),
        );
      }
    });
    websocket.addEventListener('error', () => reject(new Error('WebSocket connection failed.')));
  });
}

async function jsonRequest(url, options) {
  const response = await fetch(url, {
    ...options,
    headers: { 'content-type': 'application/json', ...options.headers },
  });
  const body = await response.json();
  if (!response.ok) {
    throw new Error(`${url} returned HTTP ${response.status}: ${JSON.stringify(body)}`);
  }
  return body;
}

async function verifyAtmosphericAsset(port) {
  const manifestResponse = await fetch(`http://127.0.0.1:${port}${assetManifestPath}`);
  if (!manifestResponse.ok) {
    throw new Error(`Atmospheric manifest returned HTTP ${manifestResponse.status}.`);
  }
  const manifest = await manifestResponse.json();
  if (manifest.assetId !== `sha256:${assetHash}`) {
    throw new Error(`Unexpected atmospheric asset id: ${manifest.assetId}`);
  }
  if (
    manifest.schema !== 2 ||
    manifest.revision !== 2 ||
    !manifest.author ||
    !manifest.license ||
    !manifest.provenance ||
    !manifest.humanReviewedAt
  ) {
    throw new Error('Atmospheric manifest is missing reviewed schema-v2 metadata.');
  }
  const expected = new Set([
    'avif:640',
    'avif:1280',
    'webp:640',
    'webp:1280',
    'jpeg:640',
    'jpeg:1280',
  ]);
  for (const variant of manifest.variants ?? []) {
    const extension = variant.format === 'jpeg' ? 'jpg' : variant.format;
    const expectedPath = `${assetHash}/scene-${variant.width}.${extension}`;
    if (!expected.delete(`${variant.format}:${variant.width}`) || variant.path !== expectedPath) {
      throw new Error(`Unexpected atmospheric variant: ${JSON.stringify(variant)}`);
    }
    const imageResponse = await fetch(`http://127.0.0.1:${port}/assets/${variant.path}`);
    if (!imageResponse.ok || imageResponse.headers.get('content-type') !== variant.mediaType) {
      throw new Error(`Atmospheric variant ${variant.path} has the wrong HTTP metadata.`);
    }
    const image = Buffer.from(await imageResponse.arrayBuffer());
    const digest = createHash('sha256').update(image).digest('hex');
    if (image.byteLength !== variant.bytes || digest !== variant.sha256) {
      throw new Error(`Atmospheric variant ${variant.path} does not match its manifest.`);
    }
  }
  if (expected.size !== 0) {
    throw new Error(`Atmospheric manifest is missing variants: ${[...expected].join(', ')}`);
  }
}

async function verifyWebBundle(port) {
  const rootResponse = await fetch(`http://127.0.0.1:${port}/`);
  if (!rootResponse.ok || !rootResponse.headers.get('content-type')?.startsWith('text/html')) {
    throw new Error('Embedded browser client index is not available.');
  }
  const html = await rootResponse.text();
  const scriptPath = html.match(/<script[^>]+src="([^"]+\.js)"/)?.[1];
  if (!scriptPath) {
    throw new Error('Embedded browser client index has no JavaScript entry point.');
  }
  const scriptResponse = await fetch(new URL(scriptPath, `http://127.0.0.1:${port}/`));
  if (
    !scriptResponse.ok ||
    !scriptResponse.headers.get('content-type')?.includes('javascript') ||
    !scriptResponse.headers.get('cache-control')?.includes('immutable')
  ) {
    throw new Error('Embedded browser client JavaScript is not served as an immutable asset.');
  }
}

function verifyTelnet(port) {
  return new Promise((resolve, reject) => {
    const socket = net.createConnection(port, '127.0.0.1');
    let received = Buffer.alloc(0);
    let state = 'access';
    const timer = setTimeout(
      () => reject(new Error(`Telnet smoke test timed out while waiting for ${state}.`)),
      5_000,
    );

    socket.on('data', (chunk) => {
      received = Buffer.concat([received, chunk]);
      const text = received.toString('utf8');
      if (state === 'access' && text.includes('Access [login/register]:')) {
        state = 'account';
        received = Buffer.alloc(0);
        socket.write('register\r\n');
      } else if (state === 'account' && text.includes('Account:')) {
        state = 'password';
        received = Buffer.alloc(0);
        socket.write(`Telnet-${Date.now()}\r\n`);
      } else if (state === 'password' && text.includes('Password:')) {
        state = 'character';
        received = Buffer.alloc(0);
        socket.write('smoke-password\r\n');
      } else if (state === 'character' && text.includes('Name your first character:')) {
        state = 'lineage';
        received = Buffer.alloc(0);
        socket.write('Telnet Explorer\r\n');
      } else if (state === 'lineage' && text.includes('Choose a number or lineage id')) {
        state = 'background';
        received = Buffer.alloc(0);
        socket.write('default\r\n');
      } else if (state === 'background' && text.includes('Choose a number or background id')) {
        state = 'base-class';
        received = Buffer.alloc(0);
        socket.write('default\r\n');
      } else if (state === 'base-class' && text.includes('Choose a number or base class id')) {
        state = 'scene';
        received = Buffer.alloc(0);
        socket.write('default\r\n');
      } else if (state === 'scene' && text.includes('Lantern Square')) {
        state = 'moved';
        received = Buffer.alloc(0);
        socket.write('north\r\n');
      } else if (state === 'moved' && text.includes('The North Gate')) {
        clearTimeout(timer);
        socket.end();
        resolve();
      }
    });
    socket.on('error', reject);
  });
}

function onceExit(child) {
  if (child.exitCode !== null) return Promise.resolve();
  return new Promise((resolve) => child.once('exit', resolve));
}

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}
