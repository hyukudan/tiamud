import assert from 'node:assert/strict';
import net from 'node:net';
import test from 'node:test';

import {
  TELNET_BYTES,
  TelnetByteParser,
  parseTelnetEndpoint,
  provisionTelnetBot,
} from './telnet-bot.mjs';

const { IAC, DONT, DO, WONT, WILL, SB, SE } = TELNET_BYTES;

test('Telnet parser keeps fragmented IAC negotiation out of application text', () => {
  const parser = new TelnetByteParser();
  assert.deepEqual(parser.push(Buffer.from([IAC])), {
    data: Buffer.alloc(0),
    response: Buffer.alloc(0),
    events: [],
  });
  assert.deepEqual(parser.push(Buffer.from([WILL])), {
    data: Buffer.alloc(0),
    response: Buffer.alloc(0),
    events: [],
  });
  const accepted = parser.push(Buffer.from([25, ...Buffer.from('look\r\n')]));
  assert.equal(accepted.data.toString(), 'look\r\n');
  assert.deepEqual([...accepted.response], [IAC, DO, 25]);
  assert.deepEqual(accepted.events, [{ kind: 'negotiation', command: WILL, option: 25 }]);

  const refused = parser.push(Buffer.from([IAC, DO, 24]));
  assert.deepEqual([...refused.response], [IAC, WONT, 24]);
  assert.equal(refused.data.length, 0);
});

test('Telnet parser unescapes text IAC and bounds fragmented subnegotiation', () => {
  const parser = new TelnetByteParser();
  assert.equal(parser.push(Buffer.from([65, IAC])).data.toString(), 'A');
  assert.deepEqual([...parser.push(Buffer.from([IAC, 66])).data], [IAC, 66]);

  parser.push(Buffer.from([IAC, SB, 201]));
  parser.push(Buffer.alloc(16 * 1024, 65));
  parser.push(Buffer.from([66, IAC]));
  const finished = parser.push(Buffer.from([SE, 67]));
  assert.equal(finished.data.toString(), 'C');
  assert.deepEqual(finished.events, [{ kind: 'subnegotiation-too-long', option: 201 }]);
});

test('Telnet endpoint parser requires an explicit plaintext host and port', () => {
  assert.deepEqual(parseTelnetEndpoint('127.0.0.1:4000'), {
    host: '127.0.0.1',
    port: 4000,
    display: '127.0.0.1:4000',
  });
  assert.throws(() => parseTelnetEndpoint('https://example.test:4000'), /plaintext host:port/);
  assert.throws(() => parseTelnetEndpoint('example.test'), /valid host and port/);
});

test('Telnet bot registers, creates, logs in, moves, speaks, reconnects, and quits cleanly', async () => {
  const sessions = [];
  let location = 'square';
  let registered = false;
  const server = net.createServer((socket) => {
    const session = { lines: [], replies: [] };
    sessions.push(session);
    const parser = new TelnetByteParser({ replyToNegotiation: false });
    let text = '';
    let state = 'access';
    socket.write(Buffer.from([IAC]));
    socket.write(Buffer.from([WILL, 25, IAC, DO]));
    socket.write(Buffer.from([24]));
    socket.write('Access [login/register]: ');
    socket.on('data', (chunk) => {
      const parsed = parser.push(chunk);
      session.replies.push(...parsed.events.filter((event) => event.kind === 'negotiation'));
      text += parsed.data.toString('utf8');
      for (;;) {
        const newline = text.indexOf('\n');
        if (newline < 0) break;
        const line = text.slice(0, newline).replace(/\r$/, '');
        text = text.slice(newline + 1);
        session.lines.push(line);
        state = handleFakeLine(socket, state, line);
      }
    });

    function handleFakeLine(connection, current, line) {
      if (current === 'access') {
        assert.equal(line, registered ? 'login' : 'register');
        connection.write('Account: ');
        return 'account';
      }
      if (current === 'account') {
        assert.match(line, /^Telnet-/);
        connection.write(Buffer.from([IAC, WILL, 1]));
        connection.write('Password: ');
        return 'password';
      }
      if (current === 'password') {
        assert.match(line, /^telnet-load-/);
        connection.write(Buffer.from([IAC, WONT, 1]));
        if (!registered) {
          registered = true;
          connection.write('Name your first character: ');
          return 'create';
        }
        connection.write('Characters:\r\n  1. Telnet Explorer\r\n');
        connection.write('Choose a number, or type a new character name: ');
        return 'choose';
      }
      if (current === 'create') {
        assert.match(line, /^Telnet Explorer/);
        connection.write('Choose a number or lineage id (type default): ');
        return 'lineage';
      }
      if (current === 'lineage') {
        assert.equal(line, 'default');
        connection.write('Choose a number or background id (type default or none): ');
        return 'background';
      }
      if (current === 'background') {
        assert.equal(line, 'default');
        connection.write('Choose a number or base class id (type default): ');
        return 'base-class';
      }
      if (current === 'base-class') {
        assert.equal(line, 'default');
        writeScene(connection, location);
        return 'playing';
      }
      if (current === 'choose') {
        assert.equal(line, '1');
        writeScene(connection, location);
        return 'playing';
      }
      if (current === 'playing') {
        if (line === 'quit') {
          connection.end('Goodbye.\r\n');
          return 'closed';
        }
        if (line === 'north') location = 'gate';
        if (line === 'south') location = 'square';
        for (const voice of ['say', 'yell', 'shout']) {
          if (line.startsWith(`${voice} `)) {
            connection.write(`You ${voice}, "${line.slice(voice.length + 1)}"\r\n`);
          }
        }
        writeScene(connection, location);
        return current;
      }
      throw new Error(`Unexpected fake Telnet state ${current} for ${JSON.stringify(line)}.`);
    }
  });

  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  const address = server.address();
  const latencies = [];
  const bot = await provisionTelnetBot(
    { host: '127.0.0.1', port: address.port },
    'test-run',
    1,
    (name) => latencies.push(name),
  );
  try {
    await bot.bootstrap();
    for (let round = 0; round < 24; round += 1) await bot.runRound();
    await bot.close();
  } finally {
    await bot.close();
    await new Promise((resolve) => server.close(resolve));
  }

  assert.equal(sessions.length, 3);
  assert.equal(
    sessions.flatMap((session) => session.lines).filter((line) => line === 'quit').length,
    3,
  );
  assert(sessions.some((session) => session.lines.includes('register')));
  assert(sessions.some((session) => session.lines.includes('login')));
  for (const voice of ['say', 'yell', 'shout']) {
    assert(sessions.flatMap((session) => session.lines).some((line) => line.startsWith(`${voice} `)));
    assert(latencies.includes(`command.${voice}`));
  }
  assert(sessions.flatMap((session) => session.replies).some((event) => event.command === DO));
  assert(latencies.includes('auth.register'));
  assert(latencies.includes('auth.login'));
});

function writeScene(socket, location) {
  socket.write(`${location === 'gate' ? 'The North Gate' : 'Lantern Square'}\r\n> `);
}
