import { performance } from 'node:perf_hooks';

const PROTOCOL = 'tiamud.v1';
const REQUEST_TIMEOUT_MS = 10_000;
const OPAQUE_ID = /^o[0-9a-f]{16}$/;
const DIRECTIONS = new Set(['north', 'east', 'south', 'west', 'up', 'down']);

export async function provisionBot(baseUrl, runId, index, recordLatency = () => {}) {
  const accountName = `Load-${runId}-${index}`;
  const password = `load-password-${runId}`;
  const auth = await timed(recordLatency, 'http.register', () =>
    jsonRequest(new URL('/api/auth/register', baseUrl), {
      method: 'POST',
      body: { name: accountName, password },
    }),
  );
  if (!isRecord(auth) || typeof auth.token !== 'string' || !/^[0-9a-f]{64}$/i.test(auth.token)) {
    throw new Error(`Registration for bot ${index} returned an invalid bearer token.`);
  }

  const created = await timed(recordLatency, 'http.character.create', () =>
    jsonRequest(new URL('/api/characters', baseUrl), {
      method: 'POST',
      token: auth.token,
      body: { name: `Load Explorer ${index}` },
    }),
  );
  const characterId = created?.character?.id;
  if (!Number.isSafeInteger(characterId) || characterId <= 0) {
    throw new Error(`Character creation for bot ${index} returned an invalid character id.`);
  }

  return new TiamudBot({
    index,
    baseUrl,
    token: auth.token,
    characterId,
    recordLatency,
  });
}

export class TiamudBot {
  #index;
  #url;
  #credentials;
  #recordLatency;
  #socket;
  #scene;
  #requestCounter = 0;
  #roundCounter = 0;
  #paceMs = 0;
  #lastActionAt = 0;

  constructor({ index, baseUrl, token, characterId, recordLatency = () => {} }) {
    this.#index = index;
    this.#url = websocketUrl(baseUrl);
    this.#credentials = { token, characterId };
    this.#recordLatency = recordLatency;
  }

  get transport() {
    return 'websocket';
  }

  set paceMs(value) {
    this.#paceMs = value;
  }

  get scene() {
    return this.#scene;
  }

  async bootstrap() {
    const initial = await this.#connectWithRetry('ws.connect');
    const initialViewId = initial.viewId;
    const north = requiredExit(initial, 'north');
    const requestId = this.#nextRequestId('idempotent-north');

    const first = await this.#command(north.command, requestId, 'command.north');
    const gate = first.scene;
    requiredExit(gate, 'south');

    const replay = await this.#command(north.command, requestId, 'command.north.replay');
    // The request result is cached, while the observer snapshot may legitimately
    // advance when another bot enters or leaves the room concurrently.
    if (
      replay.result.ok !== first.result.ok ||
      replay.result.message !== first.result.message ||
      replay.scene.viewId !== gate.viewId ||
      replay.scene.revision < gate.revision
    ) {
      throw new Error(`Bot ${this.#index} observed a non-idempotent requestId replay.`);
    }

    const afterSeq = Math.max(0, this.#socket.lastSequence - 1);
    const synced = await this.#socket.sync(afterSeq, this.#recordLatency);
    if (
      synced.afterSeq !== afterSeq ||
      synced.scene.viewId !== gate.viewId ||
      synced.scene.revision < replay.scene.revision
    ) {
      throw new Error(`Bot ${this.#index} received an inconsistent sync.snapshot.`);
    }

    await this.#socket.close();
    this.#socket = undefined;
    const reconnected = await this.#connectWithRetry('ws.reconnect');
    requiredExit(reconnected, 'south');
    if (reconnected.viewId === initialViewId || reconnected.viewId === gate.viewId) {
      throw new Error(`Bot ${this.#index} did not receive fresh opaque ids after reconnecting.`);
    }
    await this.#command('look', undefined, 'command.look.reconnect');
  }

  async runRound() {
    if (!this.#socket || !this.#scene) {
      throw new Error(`Bot ${this.#index} is not connected.`);
    }
    this.#roundCounter += 1;
    const south = requiredExit(this.#scene, 'south');
    const square = (await this.#command(south.command, undefined, 'command.south')).scene;
    requiredExit(square, 'north');
    if (this.#roundCounter % 8 === 1) {
      await this.#command(
        `say Soak pulse ${this.#index}-${this.#roundCounter}`,
        undefined,
        'command.say',
      );
    }
    await this.#command('look', undefined, 'command.look.square');
    const north = requiredExit(this.#scene, 'north');
    const gate = (await this.#command(north.command, undefined, 'command.north')).scene;
    requiredExit(gate, 'south');

    // One bot periodically drives the deterministic default NPC so multiple
    // bots do not turn expected combat ownership contention into false errors.
    if (this.#index === 1 && this.#roundCounter % 32 === 1) {
      const attack = optionalEntityAction(gate.npcs, 'attack ');
      if (attack) {
        await this.#command(attack, undefined, 'command.attack');
        // Combat resolves on logical ticks. Give defeat/recovery and NPC
        // respawn state time to settle before the next movement round.
        await delay(12_000);
      }
    }
    await this.#command('look', undefined, 'command.look.gate');

    if (this.#roundCounter % 8 === 0) {
      const requestId = this.#nextRequestId('round-idempotent-look');
      const first = await this.#command('look', requestId, 'command.look.idempotent');
      const replay = await this.#command('look', requestId, 'command.look.idempotent-replay');
      if (
        replay.result.ok !== first.result.ok ||
        replay.result.message !== first.result.message ||
        replay.scene.viewId !== first.scene.viewId ||
        replay.scene.revision < first.scene.revision
      ) {
        throw new Error(`Bot ${this.#index} observed a non-idempotent periodic replay.`);
      }

      const afterSeq = Math.max(0, this.#socket.lastSequence - 1);
      const synced = await this.#socket.sync(afterSeq, this.#recordLatency);
      if (
        synced.afterSeq !== afterSeq ||
        synced.scene.viewId !== replay.scene.viewId ||
        synced.scene.revision < replay.scene.revision
      ) {
        throw new Error(`Bot ${this.#index} received an inconsistent periodic sync.snapshot.`);
      }
    }

    if (this.#roundCounter % 16 === 0) {
      const previousViewId = this.#scene.viewId;
      await this.#socket.close();
      this.#socket = undefined;
      const reconnected = await this.#connectWithRetry('ws.periodic-reconnect');
      requiredExit(reconnected, 'south');
      if (reconnected.viewId === previousViewId) {
        throw new Error(`Bot ${this.#index} reused opaque ids after a periodic reconnect.`);
      }
    }
  }

  async close() {
    if (this.#socket) {
      await this.#socket.close();
      this.#socket = undefined;
    }
  }

  async #connectWithRetry(metric) {
    let lastError;
    for (let attempt = 0; attempt < 20; attempt += 1) {
      const socket = new StrictTiamudSocket(this.#url, this.#credentials);
      const started = performance.now();
      try {
        const scene = await socket.connect();
        this.#recordLatency(metric, performance.now() - started);
        this.#socket = socket;
        this.#scene = scene;
        return scene;
      } catch (error) {
        lastError = error;
        await socket.close().catch(() => {});
        await delay(Math.min(25 * (attempt + 1), 250));
      }
    }
    throw new Error(`Bot ${this.#index} could not connect: ${errorMessage(lastError)}`);
  }

  async #command(command, requestId, metric) {
    await this.#pace();
    const result = await this.#socket.command(
      command,
      requestId ?? this.#nextRequestId(command),
      this.#recordLatency,
      metric,
    );
    this.#lastActionAt = performance.now();
    this.#scene = result.scene;
    return result;
  }

  async #pace() {
    const remaining = this.#lastActionAt + this.#paceMs - performance.now();
    if (remaining > 0) await delay(remaining);
  }

  #nextRequestId(label) {
    this.#requestCounter += 1;
    return `load-${this.#index}-${this.#requestCounter}-${label}`.slice(0, 96);
  }
}

class StrictTiamudSocket {
  #url;
  #credentials;
  #websocket;
  #messages = [];
  #waiter;
  #terminalError;
  #closing = false;
  #lastSequence = 0;
  #sequenceGapAfter;

  constructor(url, credentials) {
    this.#url = url;
    this.#credentials = credentials;
  }

  get lastSequence() {
    return this.#lastSequence;
  }

  async connect() {
    if (typeof WebSocket !== 'function') {
      throw new Error('This harness requires the built-in WebSocket available in Node 24.');
    }
    const websocket = new WebSocket(this.#url, PROTOCOL);
    this.#websocket = websocket;
    await eventWithTimeout(websocket, 'open', REQUEST_TIMEOUT_MS, 'WebSocket open timed out');
    if (websocket.protocol !== PROTOCOL) {
      throw new Error(`Server selected unexpected WebSocket protocol ${websocket.protocol || '<none>'}.`);
    }
    websocket.addEventListener('message', (event) => this.#receive(event));
    websocket.addEventListener('error', (event) => {
      if (!this.#closing) this.#terminate(new Error(websocketErrorMessage(event)));
    });
    websocket.addEventListener('close', (event) => {
      if (!this.#closing) {
        this.#terminate(
          new Error(`WebSocket closed unexpectedly (${event.code}: ${event.reason || 'no reason'}).`),
        );
      } else {
        this.#wake();
      }
    });
    this.#send({
      v: 1,
      type: 'client.hello',
      data: {
        client: 'web',
        capabilities: {
          images: ['avif', 'webp', 'jpeg'],
          reducedMotion: true,
          audio: false,
        },
        auth: this.#credentials,
      },
    });
    const welcome = await this.#waitFor((message) => message.type === 'server.welcome');
    if (welcome.seq !== 1) throw new Error('server.welcome did not start at sequence 1.');
    return (await this.#waitFor((message) => message.type === 'scene.snapshot')).data;
  }

  async command(command, requestId, recordLatency, metric) {
    const started = performance.now();
    this.#send({
      v: 1,
      type: 'command.submit',
      data: { requestId, command },
    });
    const result = (
      await this.#waitFor(
        (message) => message.type === 'action.result' && message.data.requestId === requestId,
      )
    ).data;
    recordLatency(metric, performance.now() - started);
    if (!result.ok) {
      throw new Error(`Command ${command} failed: ${result.message ?? 'no public error'}`);
    }
    let scene = (await this.#waitFor((message) => message.type === 'scene.snapshot')).data;
    const recovered = await this.#recoverSequenceGap(recordLatency);
    if (recovered) scene = recovered.scene;
    return { result, scene };
  }

  async sync(afterSeq, recordLatency, metric = 'sync.snapshot') {
    const started = performance.now();
    this.#send({ v: 1, type: 'sync.request', data: { afterSeq } });
    const snapshot = (
      await this.#waitFor(
        (message) => message.type === 'sync.snapshot' && message.data.afterSeq === afterSeq,
      )
    ).data;
    recordLatency(metric, performance.now() - started);
    return snapshot;
  }

  async #recoverSequenceGap(recordLatency) {
    if (this.#sequenceGapAfter === undefined) return undefined;
    const afterSeq = this.#sequenceGapAfter;
    this.#sequenceGapAfter = undefined;
    return this.sync(afterSeq, recordLatency, 'sync.sequence-gap');
  }

  async close() {
    if (!this.#websocket || this.#websocket.readyState === WebSocket.CLOSED) return;
    this.#closing = true;
    const closed = eventWithTimeout(
      this.#websocket,
      'close',
      2_000,
      'WebSocket close timed out',
      false,
    );
    this.#websocket.close(1000, 'Load bot finished');
    await closed;
  }

  #receive(event) {
    try {
      if (typeof event.data !== 'string') {
        throw new Error('Server sent a non-text WebSocket frame.');
      }
      const message = parseServerMessage(event.data);
      if (message.type === 'server.heartbeat') {
        this.#send({
          v: 1,
          type: 'client.heartbeat',
          data: { nonce: message.data.nonce },
        });
        return;
      }
      if (message.seq <= this.#lastSequence) {
        throw new Error(
          `Repeated or decreasing server sequence ${message.seq}; last was ${this.#lastSequence}.`,
        );
      }
      if (message.seq !== this.#lastSequence + 1) {
        this.#sequenceGapAfter ??= this.#lastSequence;
      }
      this.#lastSequence = message.seq;
      this.#messages.push(message);
      this.#wake();
    } catch (error) {
      this.#terminate(error);
      // WHATWG clients may only initiate 1000 or application-defined close
      // codes. Keep the protocol failure in #terminalError and use a legal
      // private code so the diagnostic is not replaced by InvalidAccessError.
      this.#websocket?.close(4002, 'Strict protocol validation failed');
    }
  }

  async #waitFor(predicate) {
    const deadline = performance.now() + REQUEST_TIMEOUT_MS;
    for (;;) {
      if (this.#terminalError) throw this.#terminalError;
      while (this.#messages.length > 0) {
        const message = this.#messages.shift();
        if (message.type === 'server.notice' && message.data.level === 'error') {
          throw new Error(`Server protocol notice: ${message.data.message}`);
        }
        if (predicate(message)) return message;
      }
      const remaining = deadline - performance.now();
      if (remaining <= 0) throw new Error('Timed out waiting for a tiamud.v1 message.');
      await new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
          this.#waiter = undefined;
          reject(new Error('Timed out waiting for a tiamud.v1 message.'));
        }, remaining);
        this.#waiter = () => {
          clearTimeout(timer);
          this.#waiter = undefined;
          resolve();
        };
      });
    }
  }

  #send(value) {
    if (!this.#websocket || this.#websocket.readyState !== WebSocket.OPEN) {
      throw new Error('Cannot send through a closed WebSocket.');
    }
    this.#websocket.send(JSON.stringify(value));
  }

  #terminate(error) {
    this.#terminalError ??= error instanceof Error ? error : new Error(String(error));
    this.#wake();
  }

  #wake() {
    this.#waiter?.();
  }
}

function parseServerMessage(raw) {
  let message;
  try {
    message = JSON.parse(raw);
  } catch {
    throw new Error('Server sent malformed JSON.');
  }
  assertRecord(message, 'envelope');
  assertOnlyKeys(message, ['v', 'type', 'seq', 'id', 'data'], 'envelope');
  if (message.v !== 1 || typeof message.type !== 'string') {
    throw new Error('Server sent an invalid tiamud.v1 envelope.');
  }
  if (message.id !== undefined && (typeof message.id !== 'string' || message.id.length === 0)) {
    throw new Error('Server sent an invalid envelope id.');
  }
  assertRecord(message.data, `${message.type}.data`);

  if (message.type === 'server.heartbeat') {
    if (message.seq !== undefined) throw new Error('server.heartbeat must not be sequenced.');
    assertOnlyKeys(message.data, ['nonce', 'timeoutMs'], 'server.heartbeat.data');
    assertPositiveInteger(message.data.nonce, 'heartbeat nonce');
    assertPositiveInteger(message.data.timeoutMs, 'heartbeat timeout');
    return message;
  }
  assertPositiveInteger(message.seq, `${message.type} sequence`);

  switch (message.type) {
    case 'server.welcome':
      assertOnlyKeys(message.data, ['serverName', 'motd', 'sessionId'], 'server.welcome.data');
      optionalString(message.data.serverName, 'serverName');
      optionalString(message.data.motd, 'motd');
      optionalString(message.data.sessionId, 'sessionId');
      break;
    case 'scene.snapshot':
      validateScene(message.data);
      break;
    case 'scene.patch':
      validateScenePatch(message.data);
      break;
    case 'entity.upsert':
      validateEntityUpsert(message.data);
      break;
    case 'entity.remove':
      validateEntityRemove(message.data);
      break;
    case 'player.snapshot':
      validatePlayerState(message.data);
      break;
    case 'vitals.update':
      validateVitalsUpdate(message.data);
      break;
    case 'inventory.patch':
      validateInventoryPatch(message.data);
      break;
    case 'map.snapshot':
      validateMapSnapshot(message.data);
      break;
    case 'action.result':
      assertOnlyKeys(message.data, ['requestId', 'ok', 'message'], 'action.result.data');
      requiredString(message.data.requestId, 'action.result requestId');
      if (typeof message.data.ok !== 'boolean') throw new Error('action.result ok is not boolean.');
      if (message.data.message !== null) optionalString(message.data.message, 'action.result message');
      break;
    case 'server.notice':
      assertOnlyKeys(message.data, ['level', 'message'], 'server.notice.data');
      if (!['info', 'warning', 'error'].includes(message.data.level)) {
        throw new Error('server.notice has an invalid level.');
      }
      requiredString(message.data.message, 'server.notice message');
      break;
    case 'message.append':
      validateViewEvent(message.data);
      break;
    case 'chat.message':
      assertOnlyKeys(message.data, ['channel', 'sender', 'text'], 'chat.message.data');
      if (!['say', 'tell'].includes(message.data.channel)) {
        throw new Error('chat.message has an invalid channel.');
      }
      assertBoundedString(message.data.sender, 64, 'chat.message sender');
      assertBoundedString(message.data.text, 1024, 'chat.message text');
      break;
    case 'sync.snapshot':
      assertOnlyKeys(message.data, ['afterSeq', 'scene', 'player'], 'sync.snapshot.data');
      assertNonNegativeInteger(message.data.afterSeq, 'sync.snapshot afterSeq');
      validateScene(message.data.scene);
      validatePlayerState(message.data.player);
      break;
    default:
      throw new Error(`Server sent unknown tiamud.v1 type ${message.type}.`);
  }
  return message;
}

function validateScenePatch(patch) {
  assertRecord(patch, 'scene.patch.data');
  assertOnlyKeys(
    patch,
    ['viewId', 'revision', 'title', 'prose', 'image', 'exits', 'weather'],
    'scene.patch.data',
  );
  assertOpaqueId(patch.viewId, 'scene patch viewId');
  assertPositiveInteger(patch.revision, 'scene patch revision');
  if (Object.keys(patch).length <= 2) throw new Error('Scene patch contains no typed update.');
  if (patch.title !== undefined) assertBoundedString(patch.title, 256, 'scene patch title');
  if (patch.prose !== undefined) {
    assertArrayMax(patch.prose, 32, 'scene patch prose');
    for (const paragraph of patch.prose) assertStringMax(paragraph, 4096, 'scene patch prose');
  }
  if (patch.exits !== undefined) {
    assertArrayMax(patch.exits, 32, 'scene patch exits');
    for (const exit of patch.exits) {
      assertRecord(exit, 'scene patch exit');
      assertOnlyKeys(exit, ['id', 'label', 'command'], 'scene patch exit');
      assertOpaqueId(exit.id, 'scene patch exit id');
      assertBoundedString(exit.label, 128, 'scene patch exit label');
      assertBoundedString(exit.command, 32, 'scene patch exit command');
    }
  }
  if (patch.weather !== undefined) {
    assertRecord(patch.weather, 'scene patch weather');
    assertOnlyKeys(
      patch.weather,
      ['condition', 'day', 'hour', 'nextChangeInTicks'],
      'scene patch weather',
    );
    assertBoundedString(patch.weather.condition, 48, 'scene patch weather condition');
    assertPositiveInteger(patch.weather.day, 'scene patch weather day');
    assertNonNegativeInteger(patch.weather.hour, 'scene patch weather hour');
    assertNonNegativeInteger(patch.weather.nextChangeInTicks, 'scene patch weather next change');
  }
  if (patch.image !== undefined && patch.image !== null) {
    validateSceneImage(patch.image, 'scene patch image');
  }
}

function validateEntityBase(data, label) {
  assertOpaqueId(data.viewId, `${label} viewId`);
  assertPositiveInteger(data.revision, `${label} revision`);
  if (!['item', 'npc'].includes(data.kind)) throw new Error(`${label} has an invalid kind.`);
  assertOpaqueId(data.id, `${label} id`);
}

function validateEntityUpsert(data) {
  assertRecord(data, 'entity.upsert.data');
  assertOnlyKeys(data, ['viewId', 'revision', 'kind', 'id', 'entity'], 'entity.upsert.data');
  validateEntityBase(data, 'entity upsert');
  validateSceneEntity(data.entity, data.kind, data.kind === 'npc' ? ['health', 'maxHealth'] : []);
  if (data.entity.id !== data.id) throw new Error('Entity upsert id does not match its payload.');
}

function validateEntityRemove(data) {
  assertRecord(data, 'entity.remove.data');
  assertOnlyKeys(data, ['viewId', 'revision', 'kind', 'id'], 'entity.remove.data');
  validateEntityBase(data, 'entity remove');
}

function validateVitalsUpdate(data) {
  assertRecord(data, 'vitals.update.data');
  assertOnlyKeys(data, ['revision', 'character'], 'vitals.update.data');
  assertPositiveInteger(data.revision, 'vitals update revision');
  assertRecord(data.character, 'vitals update character');
  assertNonNegativeInteger(data.character.health, 'vitals update health');
  assertPositiveInteger(data.character.maxHealth, 'vitals update max health');
  assertArrayMax(data.character.effects, 32, 'vitals update effects');
}

function validateInventoryPatch(data) {
  assertRecord(data, 'inventory.patch.data');
  assertOnlyKeys(data, ['revision', 'inventory', 'equipment'], 'inventory.patch.data');
  assertPositiveInteger(data.revision, 'inventory patch revision');
  assertArrayMax(data.inventory, 256, 'inventory patch inventory');
  assertArrayMax(data.equipment, 3, 'inventory patch equipment');
  for (const item of data.inventory) {
    assertRecord(item, 'inventory patch item');
    assertOpaqueId(item.id, 'inventory patch item id');
    validateActions(item.actions, 'inventory patch item actions');
  }
  for (const item of data.equipment) {
    assertRecord(item, 'inventory patch equipment');
    assertOpaqueId(item.id, 'inventory patch equipment id');
    validateActions(item.actions, 'inventory patch equipment actions');
  }
}

function validateMapSnapshot(data) {
  assertRecord(data, 'map.snapshot.data');
  assertOnlyKeys(data, ['revision', 'map'], 'map.snapshot.data');
  assertPositiveInteger(data.revision, 'map snapshot revision');
  assertRecord(data.map, 'map snapshot map');
  assertOpaqueId(data.map.currentRoomId, 'map snapshot current room');
  assertArrayMax(data.map.rooms, 512, 'map snapshot rooms');
  assertArrayMax(data.map.paths, 3072, 'map snapshot paths');
}

function validateScene(scene) {
  assertRecord(scene, 'scene');
  assertOnlyKeys(
    scene,
    ['viewId', 'revision', 'title', 'prose', 'image', 'exits', 'items', 'npcs', 'weather'],
    'scene',
  );
  assertOpaqueId(scene.viewId, 'scene viewId');
  assertPositiveInteger(scene.revision, 'scene revision');
  requiredString(scene.title, 'scene title');
  assertRecord(scene.weather, 'scene weather');
  assertOnlyKeys(
    scene.weather,
    ['condition', 'day', 'hour', 'nextChangeInTicks'],
    'scene weather',
  );
  requiredString(scene.weather.condition, 'scene weather condition');
  assertPositiveInteger(scene.weather.day, 'scene weather day');
  assertNonNegativeInteger(scene.weather.hour, 'scene weather hour');
  assertNonNegativeInteger(scene.weather.nextChangeInTicks, 'scene weather next change');
  if (!Array.isArray(scene.prose) || !scene.prose.every((line) => typeof line === 'string')) {
    throw new Error('Scene prose is not a string array.');
  }
  if (!Array.isArray(scene.exits)) throw new Error('Scene exits is not an array.');
  for (const exit of scene.exits) {
    assertRecord(exit, 'scene exit');
    assertOnlyKeys(exit, ['id', 'label', 'command'], 'scene exit');
    assertOpaqueId(exit.id, 'scene exit id');
    requiredString(exit.label, 'scene exit label');
    requiredString(exit.command, 'scene exit command');
    if (!DIRECTIONS.has(exit.command)) throw new Error(`Unexpected exit command ${exit.command}.`);
  }
  if (!Array.isArray(scene.items)) throw new Error('Scene items is not an array.');
  for (const item of scene.items) validateSceneEntity(item, 'item');
  if (!Array.isArray(scene.npcs)) throw new Error('Scene npcs is not an array.');
  for (const npc of scene.npcs) {
    validateSceneEntity(npc, 'npc', ['health', 'maxHealth']);
    if (!Number.isInteger(npc.health) || !Number.isInteger(npc.maxHealth)) {
      throw new Error('NPC health is not integral.');
    }
  }
  if (scene.image !== undefined) {
    validateSceneImage(scene.image, 'scene image');
  }
  const serialized = JSON.stringify(scene);
  if (serialized.includes('demo:') || serialized.includes('tiamud:')) {
    throw new Error('Authorized scene leaked a stable WorldPack identifier.');
  }
}

function validateSceneImage(image, label) {
  assertRecord(image, label);
  assertOnlyKeys(
    image,
    ['alt', 'localizedAlt', 'manifest', 'avif', 'webp', 'jpeg', 'width', 'height'],
    label,
  );
  assertBoundedString(image.alt, 1024, `${label} alt`);
  if (image.localizedAlt !== undefined) {
    assertRecord(image.localizedAlt, `${label} localizedAlt`);
    const entries = Object.entries(image.localizedAlt);
    if (entries.length === 0 || entries.length > 16) {
      throw new Error(`${label} localizedAlt is outside its 16-entry bound.`);
    }
    for (const [locale, text] of entries) {
      if (!/^[a-z0-9]+(?:-[a-z0-9]+)*$/i.test(locale) || locale.length > 35) {
        throw new Error(`${label} localizedAlt contains an invalid locale.`);
      }
      assertBoundedString(text, 512, `${label} localizedAlt text`);
    }
  }
  for (const key of ['manifest', 'avif', 'webp', 'jpeg']) {
    optionalString(image[key], `${label} ${key}`);
  }
  for (const key of ['width', 'height']) {
    if (image[key] !== undefined) assertPositiveInteger(image[key], `${label} ${key}`);
  }
}

function validateSceneEntity(entity, kind, extraKeys = []) {
  assertRecord(entity, `scene ${kind}`);
  assertOnlyKeys(entity, ['id', 'label', 'description', 'actions', ...extraKeys], `scene ${kind}`);
  assertOpaqueId(entity.id, `scene ${kind} id`);
  requiredString(entity.label, `scene ${kind} label`);
  if (typeof entity.description !== 'string') throw new Error(`Scene ${kind} description is invalid.`);
  if (!Array.isArray(entity.actions)) throw new Error(`Scene ${kind} actions is not an array.`);
  for (const action of entity.actions) {
    assertRecord(action, `scene ${kind} action`);
    assertOnlyKeys(action, ['label', 'command'], `scene ${kind} action`);
    requiredString(action.label, `scene ${kind} action label`);
    requiredString(action.command, `scene ${kind} action command`);
    const target = action.command.split(/\s+/).at(-1);
    if (action.command.includes(' ') && !OPAQUE_ID.test(target)) {
      throw new Error(`Scene ${kind} action does not target an opaque id.`);
    }
  }
}

function validatePlayerState(player) {
  assertRecord(player, 'player');
  assertOnlyKeys(
    player,
    [
      'revision', 'character', 'inventory', 'equipment', 'map', 'quests', 'party',
      'partyInvites', 'economy', 'social',
    ],
    'player',
  );
  assertPositiveInteger(player.revision, 'player revision');

  const character = player.character;
  assertRecord(character, 'player character');
  assertOnlyKeys(
    character,
    [
      'name', 'level', 'experience', 'nextLevelExperience', 'health', 'maxHealth',
      'strength', 'defence', 'skills', 'effects', 'recoveryInTicks',
    ],
    'player character',
  );
  assertBoundedString(character.name, 64, 'player character name');
  assertPositiveInteger(character.level, 'player character level');
  assertNonNegativeInteger(character.experience, 'player character experience');
  assertPositiveInteger(character.nextLevelExperience, 'player next level experience');
  assertNonNegativeInteger(character.health, 'player health');
  assertPositiveInteger(character.maxHealth, 'player max health');
  if (character.health > character.maxHealth) throw new Error('Player health exceeds max health.');
  assertPositiveInteger(character.strength, 'player strength');
  assertNonNegativeInteger(character.defence, 'player defence');
  assertArrayMax(character.skills, 32, 'player skills');
  for (const skill of character.skills) {
    assertRecord(skill, 'player skill');
    assertOnlyKeys(skill, ['id', 'label', 'rank'], 'player skill');
    assertContentId(skill.id, 'player skill id');
    assertBoundedString(skill.label, 128, 'player skill label');
    assertNonNegativeInteger(skill.rank, 'player skill rank');
    if (skill.rank > 100) throw new Error('Player skill rank exceeds its bound.');
  }
  if (character.recoveryInTicks !== null && character.recoveryInTicks !== undefined) {
    assertNonNegativeInteger(character.recoveryInTicks, 'player recovery ticks');
  }
  assertArrayMax(character.effects, 32, 'player effects');
  for (const effect of character.effects) {
    assertRecord(effect, 'player effect');
    assertOnlyKeys(effect, ['kind', 'magnitude', 'remainingTicks'], 'player effect');
    if (effect.kind !== 'regeneration' && effect.kind !== 'strength') {
      throw new Error('Player effect kind is invalid.');
    }
    assertPositiveInteger(effect.magnitude, 'player effect magnitude');
    assertPositiveInteger(effect.remainingTicks, 'player effect remaining ticks');
  }

  assertArrayMax(player.inventory, 256, 'player inventory');
  for (const item of player.inventory) {
    assertRecord(item, 'player inventory item');
    assertOnlyKeys(item, ['id', 'label', 'description', 'actions'], 'player inventory item');
    assertOpaqueId(item.id, 'player inventory item id');
    assertBoundedString(item.label, 256, 'player inventory item label');
    assertStringMax(item.description, 4096, 'player inventory item description');
    validateActions(item.actions, 'player inventory item actions');
  }

  assertArrayMax(player.equipment, 3, 'player equipment');
  for (const item of player.equipment) {
    assertRecord(item, 'player equipment item');
    assertOnlyKeys(item, ['id', 'slot', 'label', 'actions'], 'player equipment item');
    assertOpaqueId(item.id, 'player equipment item id');
    if (!['weapon', 'body', 'trinket'].includes(item.slot)) {
      throw new Error('Player equipment slot is invalid.');
    }
    assertBoundedString(item.label, 256, 'player equipment item label');
    validateActions(item.actions, 'player equipment item actions');
  }

  const map = player.map;
  assertRecord(map, 'player map');
  assertOnlyKeys(map, ['currentRoomId', 'rooms', 'paths'], 'player map');
  assertOpaqueId(map.currentRoomId, 'player current room id');
  assertArrayMax(map.rooms, 512, 'player map rooms');
  const roomIds = new Set();
  let currentRooms = 0;
  for (const room of map.rooms) {
    assertRecord(room, 'player map room');
    assertOnlyKeys(room, ['id', 'title', 'current'], 'player map room');
    assertOpaqueId(room.id, 'player map room id');
    assertBoundedString(room.title, 256, 'player map room title');
    assertBoolean(room.current, 'player map room current');
    if (roomIds.has(room.id)) throw new Error('Player map contains a duplicate room id.');
    roomIds.add(room.id);
    if (room.current) currentRooms += 1;
  }
  if (!roomIds.has(map.currentRoomId) || currentRooms !== 1 ||
      !map.rooms.some((room) => room.id === map.currentRoomId && room.current)) {
    throw new Error('Player map current room is inconsistent.');
  }
  assertArrayMax(map.paths, 3072, 'player map paths');
  for (const path of map.paths) {
    assertRecord(path, 'player map path');
    assertOnlyKeys(path, ['from', 'direction', 'to'], 'player map path');
    assertOpaqueId(path.from, 'player map path source');
    assertBoundedString(path.direction, 16, 'player map path direction');
    if (!roomIds.has(path.from)) throw new Error('Player map path source is unknown.');
    if (path.to !== undefined) {
      assertOpaqueId(path.to, 'player map path target');
      if (!roomIds.has(path.to)) throw new Error('Player map path target is unknown.');
    }
  }

  assertArrayMax(player.quests, 32, 'player quests');
  for (const quest of player.quests) {
    assertRecord(quest, 'player quest');
    assertOnlyKeys(
      quest,
      ['id', 'label', 'summary', 'status', 'current', 'target', 'actions'],
      'player quest',
    );
    assertContentId(quest.id, 'player quest id');
    assertBoundedString(quest.label, 128, 'player quest label');
    assertBoundedString(quest.summary, 1024, 'player quest summary');
    if (!['available', 'active', 'readyToClaim', 'completed'].includes(quest.status)) {
      throw new Error('Player quest status is invalid.');
    }
    assertNonNegativeInteger(quest.current, 'player quest current progress');
    assertPositiveInteger(quest.target, 'player quest target');
    if (quest.current > quest.target) throw new Error('Player quest progress exceeds its target.');
    validateActions(quest.actions, 'player quest actions');
  }

  if (player.party !== undefined && player.party !== null) {
    const party = player.party;
    assertRecord(party, 'player party');
    assertOnlyKeys(party, ['leader', 'members', 'actions'], 'player party');
    assertBoolean(party.leader, 'player party leadership');
    assertArrayMax(party.members, 6, 'player party members');
    if (party.members.length === 0) throw new Error('Player party has no members.');
    let leaders = 0;
    for (const member of party.members) {
      assertRecord(member, 'player party member');
      assertOnlyKeys(member, ['id', 'name', 'leader', 'online'], 'player party member');
      assertOpaqueId(member.id, 'player party member id');
      assertBoundedString(member.name, 64, 'player party member name');
      assertBoolean(member.leader, 'player party member leadership');
      assertBoolean(member.online, 'player party member online state');
      if (member.leader) leaders += 1;
    }
    if (leaders !== 1) throw new Error('Player party must have exactly one leader.');
    validateActions(party.actions, 'player party actions');
  }

  assertArrayMax(player.partyInvites, 12, 'player party invites');
  for (const invite of player.partyInvites) {
    assertRecord(invite, 'player party invite');
    assertOnlyKeys(invite, ['leaderName', 'expiresInTicks', 'actions'], 'player party invite');
    assertBoundedString(invite.leaderName, 64, 'player party invite leader');
    assertNonNegativeInteger(invite.expiresInTicks, 'player party invite expiry');
    validateActions(invite.actions, 'player party invite actions');
  }

  const economy = player.economy;
  assertRecord(economy, 'player economy');
  assertOnlyKeys(economy, ['coins', 'commodities', 'shopAvailable', 'shop'], 'player economy');
  assertNonNegativeInteger(economy.coins, 'player coins');
  assertBoolean(economy.shopAvailable, 'player shop availability');
  assertArrayMax(economy.commodities, 32, 'player commodities');
  for (const commodity of economy.commodities) {
    assertRecord(commodity, 'player commodity');
    assertOnlyKeys(commodity, ['id', 'label', 'quantity'], 'player commodity');
    assertContentId(commodity.id, 'player commodity id');
    assertBoundedString(commodity.label, 128, 'player commodity label');
    assertPositiveInteger(commodity.quantity, 'player commodity quantity');
    if (commodity.quantity > 99) throw new Error('Player commodity quantity exceeds its bound.');
  }
  assertArrayMax(economy.shop, 32, 'player shop');
  for (const item of economy.shop) {
    assertRecord(item, 'player shop item');
    assertOnlyKeys(
      item,
      ['id', 'label', 'buyPrice', 'sellPrice', 'stock', 'owned', 'actions'],
      'player shop item',
    );
    assertContentId(item.id, 'player shop item id');
    assertBoundedString(item.label, 128, 'player shop item label');
    assertPositiveInteger(item.buyPrice, 'player shop item buy price');
    assertPositiveInteger(item.sellPrice, 'player shop item sell price');
    assertNonNegativeInteger(item.stock, 'player shop item stock');
    assertNonNegativeInteger(item.owned, 'player shop item owned');
    if (item.stock > 100 || item.owned > 99) throw new Error('Player shop item exceeds its bound.');
    validateActions(item.actions, 'player shop item actions');
  }
  if (!economy.shopAvailable && economy.shop.length > 0) {
    throw new Error('Player shop contains items while unavailable.');
  }

  const social = player.social;
  assertRecord(social, 'player social state');
  assertOnlyKeys(social, ['online'], 'player social state');
  assertArrayMax(social.online, 256, 'player online characters');
  for (const online of social.online) {
    assertRecord(online, 'player online character');
    assertOnlyKeys(online, ['id', 'name', 'selfCharacter'], 'player online character');
    assertOpaqueId(online.id, 'player online character id');
    assertBoundedString(online.name, 64, 'player online character name');
    assertBoolean(online.selfCharacter, 'player online character self marker');
  }

  const serialized = JSON.stringify(player);
  if (serialized.includes('demo:') || serialized.includes('tiamud:')) {
    throw new Error('Authorized player state leaked a stable WorldPack identifier.');
  }
}

function validateActions(actions, label) {
  assertArrayMax(actions, 8, label);
  for (const action of actions) {
    assertRecord(action, `${label} entry`);
    assertOnlyKeys(action, ['label', 'command'], `${label} entry`);
    assertBoundedString(action.label, 128, `${label} label`);
    assertBoundedString(action.command, 1024, `${label} command`);
  }
}

function validateViewEvent(event) {
  assertRecord(event, 'message.append.data');
  assertOnlyKeys(
    event,
    ['revision', 'kind', 'text', 'actorId', 'targetId', 'itemId'],
    'message.append.data',
  );
  assertPositiveInteger(event.revision, 'message revision');
  if (!['characterMoved', 'itemTaken', 'itemDropped', 'gameplay', 'adventure'].includes(event.kind)) {
    throw new Error(`Unknown message.append kind ${event.kind}.`);
  }
  requiredString(event.text, 'message text');
  assertOpaqueId(event.actorId, 'message actorId');
  if (event.targetId !== undefined) assertOpaqueId(event.targetId, 'message targetId');
  if (event.itemId !== undefined) assertOpaqueId(event.itemId, 'message itemId');
}

function requiredExit(scene, command) {
  const exit = scene.exits.find((candidate) => candidate.command === command);
  if (!exit) throw new Error(`Scene ${scene.title} has no authorized ${command} exit.`);
  assertOpaqueId(exit.id, `${command} exit id`);
  return exit;
}

function optionalEntityAction(entities, prefix) {
  for (const entity of entities) {
    const action = entity.actions.find((candidate) => candidate.command.startsWith(prefix));
    if (action) return action.command;
  }
  return undefined;
}

async function jsonRequest(url, { method, body, token }) {
  const response = await fetch(url, {
    method,
    signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
    headers: {
      'content-type': 'application/json',
      ...(token ? { authorization: `Bearer ${token}` } : {}),
    },
    body: JSON.stringify(body),
  });
  const text = await response.text();
  let parsed;
  try {
    parsed = text ? JSON.parse(text) : undefined;
  } catch {
    throw new Error(`${url} returned non-JSON HTTP ${response.status}.`);
  }
  if (!response.ok) {
    throw new Error(`${url} returned HTTP ${response.status}: ${JSON.stringify(parsed)}`);
  }
  return parsed;
}

function websocketUrl(baseUrl) {
  const url = new URL('/ws', baseUrl);
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  return url;
}

async function timed(recordLatency, name, operation) {
  const started = performance.now();
  const result = await operation();
  recordLatency(name, performance.now() - started);
  return result;
}

function eventWithTimeout(target, eventName, timeoutMs, message, rejectOnError = true) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      cleanup();
      reject(new Error(message));
    }, timeoutMs);
    const onEvent = (event) => {
      cleanup();
      resolve(event);
    };
    const onError = (event) => {
      cleanup();
      reject(new Error(websocketErrorMessage(event)));
    };
    const cleanup = () => {
      clearTimeout(timer);
      target.removeEventListener(eventName, onEvent);
      if (eventName !== 'error' && rejectOnError) target.removeEventListener('error', onError);
    };
    target.addEventListener(eventName, onEvent, { once: true });
    if (eventName !== 'error' && rejectOnError) {
      target.addEventListener('error', onError, { once: true });
    }
  });
}

function assertRecord(value, label) {
  if (!isRecord(value)) throw new Error(`${label} is not an object.`);
}

function isRecord(value) {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function assertOnlyKeys(value, allowed, label) {
  const keys = new Set(allowed);
  const unknown = Object.keys(value).filter((key) => !keys.has(key));
  if (unknown.length > 0) throw new Error(`${label} has unknown keys: ${unknown.join(', ')}.`);
}

function assertOpaqueId(value, label) {
  if (typeof value !== 'string' || !OPAQUE_ID.test(value)) {
    throw new Error(`${label} is not an opaque per-view id.`);
  }
}

function assertContentId(value, label) {
  if (typeof value !== 'string' || !/^[a-z0-9][a-z0-9-]{0,63}$/.test(value)) {
    throw new Error(`${label} is not a bounded content id.`);
  }
}

function requiredString(value, label) {
  if (typeof value !== 'string' || value.length === 0) throw new Error(`${label} is invalid.`);
}

function assertBoundedString(value, maximum, label) {
  requiredString(value, label);
  if (value.length > maximum) throw new Error(`${label} exceeds ${maximum} characters.`);
}

function assertStringMax(value, maximum, label) {
  if (typeof value !== 'string' || value.length > maximum) {
    throw new Error(`${label} is invalid or exceeds ${maximum} characters.`);
  }
}

function assertArrayMax(value, maximum, label) {
  if (!Array.isArray(value) || value.length > maximum) {
    throw new Error(`${label} is not an array within its ${maximum}-entry bound.`);
  }
}

function assertBoolean(value, label) {
  if (typeof value !== 'boolean') throw new Error(`${label} is not boolean.`);
}

function optionalString(value, label) {
  if (value !== undefined) requiredString(value, label);
}

function assertPositiveInteger(value, label) {
  if (!Number.isSafeInteger(value) || value <= 0) throw new Error(`${label} is not positive.`);
}

function assertNonNegativeInteger(value, label) {
  if (!Number.isSafeInteger(value) || value < 0) throw new Error(`${label} is negative.`);
}

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

function websocketErrorMessage(event) {
  const detail = event?.error?.message ?? event?.message;
  return detail ? `WebSocket transport error: ${detail}` : 'WebSocket transport error.';
}
