import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';

const root = path.resolve(import.meta.dirname, '..');
const temporary = await mkdtemp(path.join(os.tmpdir(), 'project-envy-smoke-'));
const httpPort = await reservePort();
const telnetPort = await reservePort();
let output = '';

const server = spawn('cargo', ['run', '--locked', '-p', 'mud-server'], {
  cwd: root,
  env: {
    ...process.env,
    MUD_DB_PATH: path.join(temporary, 'smoke.db'),
    MUD_HTTP_ADDR: `127.0.0.1:${httpPort}`,
    MUD_TELNET_ADDR: `127.0.0.1:${telnetPort}`,
  },
  stdio: ['ignore', 'pipe', 'pipe'],
});

server.stdout.on('data', (chunk) => (output += chunk));
server.stderr.on('data', (chunk) => (output += chunk));

try {
  await waitUntilReady(httpPort, server);
  await Promise.all([verifyBrowser(httpPort), verifyTelnet(telnetPort)]);
  console.log('Smoke test passed: HTTP, mud.v1 WebSocket, and Telnet share the demo world.');
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
        if (status.status === 'ready' && status.storage === 'sqlite') return;
      }
    } catch {
      // The listener may still be starting.
    }
    await delay(50);
  }
  throw new Error('Server did not become ready within fifteen seconds.');
}

function verifyBrowser(port) {
  return new Promise((resolve, reject) => {
    const websocket = new WebSocket(`ws://127.0.0.1:${port}/ws`, 'mud.v1');
    let moved = false;
    const timer = setTimeout(() => reject(new Error('WebSocket smoke test timed out.')), 5_000);

    websocket.addEventListener('open', () => {
      websocket.send(
        JSON.stringify({
          v: 1,
          type: 'client.hello',
          data: {
            client: 'web',
            capabilities: { images: ['avif', 'webp', 'jpeg'], reducedMotion: true },
          },
        }),
      );
    });
    websocket.addEventListener('message', (event) => {
      const message = JSON.parse(event.data);
      if (message.type !== 'scene.snapshot') return;
      if (!moved) {
        if (message.data.viewId !== 'demo:lantern-square') {
          reject(new Error(`Unexpected initial browser scene: ${message.data.viewId}`));
          return;
        }
        moved = true;
        websocket.send(
          JSON.stringify({
            v: 1,
            type: 'command.submit',
            data: { requestId: 'smoke-browser-1', command: 'north' },
          }),
        );
      } else if (message.data.viewId === 'demo:north-gate') {
        clearTimeout(timer);
        websocket.close();
        resolve();
      }
    });
    websocket.addEventListener('error', () => reject(new Error('WebSocket connection failed.')));
  });
}

function verifyTelnet(port) {
  return new Promise((resolve, reject) => {
    const socket = net.createConnection(port, '127.0.0.1');
    let received = Buffer.alloc(0);
    let moved = false;
    const timer = setTimeout(() => reject(new Error('Telnet smoke test timed out.')), 5_000);

    socket.on('data', (chunk) => {
      received = Buffer.concat([received, chunk]);
      if (!moved && received.includes(Buffer.from('Lantern Square'))) {
        moved = true;
        socket.write('north\r\n');
      }
      if (moved && received.includes(Buffer.from('The North Gate'))) {
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
