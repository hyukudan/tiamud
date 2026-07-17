import { performance } from 'node:perf_hooks';

const PROTOCOL = 'tiamud.v1';
const REQUEST_TIMEOUT_MS = 10_000;
const OPAQUE_ID = /^o[0-9a-f]{16}$/;
const DIRECTIONS = new Set(['north', 'east', 'south', 'west', 'up', 'down']);
const ITEM_RARITIES = new Set(['common', 'uncommon', 'rare', 'epic', 'legendary']);
const EQUIPMENT_SLOTS = new Set(['weapon', 'body', 'trinket']);
const MAX_ITEM_LEVEL = 1_000;
const MAX_ITEM_WEIGHT_GRAMS = 1_000_000;
const MAX_ITEM_SLOT_COST = 100;
const MAX_ITEM_STACK_UNITS = 1_000;
const MAX_ITEM_TOTAL_WEIGHT_GRAMS = 65_536_000_000;
const MAX_INVENTORY_SLOTS = 1_000;
const MAX_INVENTORY_WEIGHT_GRAMS = 1_000_000;
const MAX_SCENE_PRESENTATION_ENTITIES = 12;
const MAX_SCENE_PRESENTATION_POSES = 32;
const MAX_ENTITY_SOURCE_REFERENCES = 2;
const SOCIAL_CHANNELS = new Set([
  'announcement', 'say', 'tell', 'party', 'general', 'newbie', 'staff',
]);
const MEDIA_CHANNELS = new Set(['music', 'ambience', 'effects']);

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
      throw new Error(`Bot ${this.#index} received an inconsistent sync recovery pair.`);
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
    const south = optionalExit(this.#scene, 'south');
    const square = south
      ? (await this.#command(south.command, undefined, 'command.south')).scene
      : this.#scene;
    requiredExit(square, 'north');
    if (this.#roundCounter % 8 === 1) {
      await this.#command(
        `say Soak pulse ${this.#index}-${this.#roundCounter}`,
        undefined,
        'command.say',
      );
    }
    await this.#command('look', undefined, 'command.look.square');
    const north = optionalExit(this.#scene, 'north');
    const gate = north
      ? (await this.#command(north.command, undefined, 'command.north')).scene
      : this.#scene;
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
    const returnNorth = optionalExit(this.#scene, 'north');
    if (returnNorth) {
      await this.#command(returnNorth.command, undefined, 'command.north.after-recovery');
    }
    requiredExit(this.#scene, 'south');

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
        throw new Error(`Bot ${this.#index} received an inconsistent periodic sync recovery pair.`);
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

  async sync(afterSeq, recordLatency, metric = 'sync.recovery') {
    const started = performance.now();
    this.#send({ v: 1, type: 'sync.request', data: { afterSeq } });
    const sceneHalf = (
      await this.#waitFor(
        (message) => message.type === 'sync.scene' && message.data.afterSeq === afterSeq,
      )
    ).data;
    const playerHalf = (
      await this.#waitFor(
        (message) => message.type === 'sync.player' &&
          message.data.afterSeq === afterSeq &&
          message.data.syncId === sceneHalf.syncId,
      )
    ).data;
    // The correlated player half makes every earlier sequence gap irrelevant:
    // the pair is the new complete authoritative baseline. A gap caused by
    // coalescing state frames immediately before the recovery itself must not
    // trigger a redundant second recovery on the next command.
    this.#sequenceGapAfter = undefined;
    recordLatency(metric, performance.now() - started);
    return {
      syncId: sceneHalf.syncId,
      afterSeq,
      scene: sceneHalf.scene,
      player: playerHalf.player,
    };
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

export function parseServerMessage(raw) {
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
    case 'container.contents':
      validateContainerContents(message.data);
      break;
    case 'trade.contents':
      validateTradeContents(message.data);
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
      assertOnlyKeys(message.data, ['level', 'code', 'message'], 'server.notice.data');
      if (!['info', 'warning', 'error'].includes(message.data.level)) {
        throw new Error('server.notice has an invalid level.');
      }
      if (message.data.code !== undefined) {
        assertBoundedString(message.data.code, 64, 'server.notice code');
        if (!/^[a-z][a-z0-9_.-]*$/.test(message.data.code)) {
          throw new Error('server.notice code is invalid.');
        }
      }
      assertBoundedString(message.data.message, 1024, 'server.notice message');
      break;
    case 'message.append':
      validateViewEvent(message.data);
      break;
    case 'chat.message':
      assertOnlyKeys(message.data, ['channel', 'sender', 'text'], 'chat.message.data');
      if (!SOCIAL_CHANNELS.has(message.data.channel)) {
        throw new Error('chat.message has an invalid channel.');
      }
      assertBoundedString(message.data.sender, 64, 'chat.message sender');
      assertBoundedString(message.data.text, 512, 'chat.message text');
      break;
    case 'media.play':
      assertOnlyKeys(message.data, ['channel', 'url', 'loop'], 'media.play.data');
      validateMediaChannel(message.data.channel, 'media.play channel');
      validateAudioUrl(message.data.url, 'media.play url');
      assertBoolean(message.data.loop, 'media.play loop');
      break;
    case 'media.stop':
      assertOnlyKeys(message.data, ['channel'], 'media.stop.data');
      if (message.data.channel !== undefined) {
        validateMediaChannel(message.data.channel, 'media.stop channel');
      }
      break;
    case 'sync.scene':
      assertOnlyKeys(message.data, ['syncId', 'afterSeq', 'scene'], 'sync.scene.data');
      validateSyncId(message.data.syncId, 'sync.scene syncId');
      assertNonNegativeInteger(message.data.afterSeq, 'sync.scene afterSeq');
      validateScene(message.data.scene);
      break;
    case 'sync.player':
      assertOnlyKeys(
        message.data,
        ['syncId', 'afterSeq', 'player', 'complete'],
        'sync.player.data',
      );
      validateSyncId(message.data.syncId, 'sync.player syncId');
      assertNonNegativeInteger(message.data.afterSeq, 'sync.player afterSeq');
      if (message.data.complete !== true) throw new Error('sync.player is not complete.');
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
  validatePlayerCharacter(data.character);
}

function validateInventoryPatch(data) {
  assertRecord(data, 'inventory.patch.data');
  assertOnlyKeys(
    data,
    ['revision', 'inventory', 'equipment', 'inventoryCapacity'],
    'inventory.patch.data',
  );
  assertPositiveInteger(data.revision, 'inventory patch revision');
  validatePlayerInventory(
    data.inventory,
    data.equipment,
    data.inventoryCapacity,
    'inventory patch',
  );
}

function validateContainerContents(data) {
  assertRecord(data, 'container.contents.data');
  assertOnlyKeys(
    data,
    ['container', 'items', 'page', 'totalPages', 'capacity'],
    'container.contents.data',
  );
  assertOpaqueId(data.container, 'container contents selector');
  assertPositiveInteger(data.page, 'container contents page');
  assertPositiveInteger(data.totalPages, 'container contents total pages');
  if (data.page > data.totalPages || data.totalPages > 0xffff) {
    throw new Error('Container contents page is invalid.');
  }
  assertArrayMax(data.items, 20, 'container contents items');
  const ids = new Set();
  for (const item of data.items) {
    assertRecord(item, 'container contents item');
    assertOnlyKeys(
      item,
      [
        'id', 'label', 'rarity', 'itemLevel', 'weightGrams', 'quantity', 'stackSlots',
        'availableQuantity', 'reservedForTrade', 'totalWeightGrams', 'containerCapacity',
        'transferPolicy', 'boundToYou', 'canGive', 'canTrade', 'actions',
      ],
      'container contents item',
    );
    assertOpaqueId(item.id, 'container contents item id');
    if (ids.has(item.id)) throw new Error('Container contents contains a duplicate item id.');
    ids.add(item.id);
    assertBoundedString(item.label, 256, 'container contents item label');
    validateItemMetadata(item, 'container contents item');
    assertBoolean(item.canGive, 'container contents item give availability');
    if (item.canGive) throw new Error('Container contents item cannot be given directly.');
    validateItemTradeAvailability(item, 'container contents item');
    validateActions(item.actions, 'container contents item actions');
  }
  validateCapacityShape(data.capacity, 'container contents capacity');
}

function validateTradeContents(data) {
  assertRecord(data, 'trade.contents.data');
  assertOnlyKeys(
    data,
    ['tradeId', 'container', 'items', 'page', 'totalPages', 'capacity'],
    'trade.contents.data',
  );
  assertOpaqueId(data.tradeId, 'trade contents trade id');
  validateContainerContents({
    container: data.container,
    items: data.items,
    page: data.page,
    totalPages: data.totalPages,
    capacity: data.capacity,
  });
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
    [
      'viewId', 'revision', 'title', 'prose', 'image', 'audio', 'presentation',
      'exits', 'items', 'npcs', 'weather',
    ],
    'scene',
  );
  assertOpaqueId(scene.viewId, 'scene viewId');
  assertPositiveInteger(scene.revision, 'scene revision');
  assertBoundedString(scene.title, 256, 'scene title');
  assertRecord(scene.weather, 'scene weather');
  assertOnlyKeys(
    scene.weather,
    ['condition', 'day', 'hour', 'nextChangeInTicks'],
    'scene weather',
  );
  assertBoundedString(scene.weather.condition, 48, 'scene weather condition');
  assertPositiveInteger(scene.weather.day, 'scene weather day');
  assertNonNegativeInteger(scene.weather.hour, 'scene weather hour');
  if (scene.weather.hour > 23) throw new Error('Scene weather hour exceeds 23.');
  assertNonNegativeInteger(scene.weather.nextChangeInTicks, 'scene weather next change');
  assertArrayMax(scene.prose, 32, 'scene prose');
  for (const line of scene.prose) assertStringMax(line, 4096, 'scene prose line');
  assertArrayMax(scene.exits, 32, 'scene exits');
  const exitIds = new Set();
  const exitCommands = new Set();
  for (const exit of scene.exits) {
    assertRecord(exit, 'scene exit');
    assertOnlyKeys(exit, ['id', 'label', 'command'], 'scene exit');
    assertOpaqueId(exit.id, 'scene exit id');
    assertBoundedString(exit.label, 128, 'scene exit label');
    assertBoundedString(exit.command, 32, 'scene exit command');
    if (!DIRECTIONS.has(exit.command)) throw new Error(`Unexpected exit command ${exit.command}.`);
    if (exitIds.has(exit.id) || exitCommands.has(exit.command)) {
      throw new Error('Scene contains a duplicate exit id or command.');
    }
    exitIds.add(exit.id);
    exitCommands.add(exit.command);
  }
  if (!Array.isArray(scene.items)) throw new Error('Scene items is not an array.');
  assertArrayMax(scene.items, 256, 'scene items');
  const itemIds = new Set();
  for (const item of scene.items) {
    validateSceneEntity(item, 'item');
    if (itemIds.has(item.id)) throw new Error('Scene contains a duplicate item id.');
    itemIds.add(item.id);
  }
  if (!Array.isArray(scene.npcs)) throw new Error('Scene npcs is not an array.');
  assertArrayMax(scene.npcs, 256, 'scene npcs');
  const npcIds = new Set();
  for (const npc of scene.npcs) {
    validateSceneEntity(npc, 'npc', ['health', 'maxHealth']);
    assertNonNegativeInteger(npc.health, 'scene NPC health');
    assertPositiveInteger(npc.maxHealth, 'scene NPC max health');
    if (npc.health > npc.maxHealth || npc.maxHealth > 1_000_000) {
      throw new Error('Scene NPC health is inconsistent.');
    }
    if (npcIds.has(npc.id)) throw new Error('Scene contains a duplicate NPC id.');
    npcIds.add(npc.id);
  }
  if (scene.image !== undefined) {
    validateSceneImage(scene.image, 'scene image');
  }
  if (scene.audio !== undefined) validateSceneAudio(scene.audio, 'scene audio');
  if (scene.presentation !== undefined) {
    validateScenePresentation(scene.presentation, new Set([...itemIds, ...npcIds]));
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

function validateSceneAudio(audio, label) {
  assertRecord(audio, label);
  assertOnlyKeys(audio, ['channel', 'url', 'loop'], label);
  validateMediaChannel(audio.channel, `${label} channel`);
  validateAudioUrl(audio.url, `${label} url`);
  assertBoolean(audio.loop, `${label} loop`);
}

function validateScenePresentation(presentation, visibleEntityIds) {
  assertRecord(presentation, 'scene presentation');
  assertOnlyKeys(presentation, ['entities'], 'scene presentation');
  assertArrayMax(
    presentation.entities,
    MAX_SCENE_PRESENTATION_ENTITIES,
    'scene presentation entities',
  );
  if (presentation.entities.length === 0) {
    throw new Error('Scene presentation contains no entities.');
  }
  const ids = new Set();
  let poseCount = 0;
  for (const entity of presentation.entities) {
    assertRecord(entity, 'scene presentation entity');
    assertOnlyKeys(
      entity,
      [
        'id', 'alt', 'decorative', 'footprint', 'preferredAnchor', 'priority',
        'layer', 'poses',
      ],
      'scene presentation entity',
    );
    assertOpaqueId(entity.id, 'scene presentation entity id');
    if (!visibleEntityIds.has(entity.id) || ids.has(entity.id)) {
      throw new Error('Scene presentation references an invisible or duplicate entity.');
    }
    ids.add(entity.id);
    assertBoolean(entity.decorative, 'scene presentation decorative marker');
    validateVisualText(entity.alt, 256, entity.decorative, 'scene presentation alt');
    assertRecord(entity.footprint, 'scene presentation footprint');
    assertOnlyKeys(entity.footprint, ['width', 'height'], 'scene presentation footprint');
    assertFiniteRange(entity.footprint.width, 0.02, 0.6, 'scene presentation width');
    assertFiniteRange(entity.footprint.height, 0.02, 0.8, 'scene presentation height');
    if (entity.preferredAnchor !== undefined) {
      assertRecord(entity.preferredAnchor, 'scene presentation preferred anchor');
      assertOnlyKeys(entity.preferredAnchor, ['x', 'y'], 'scene presentation preferred anchor');
      assertFiniteRange(entity.preferredAnchor.x, 0, 1, 'scene presentation anchor x');
      assertFiniteRange(entity.preferredAnchor.y, 0.2, 1, 'scene presentation anchor y');
    }
    assertSignedInteger(entity.priority, -100, 100, 'scene presentation priority');
    if (!['midground', 'foreground', 'overlay'].includes(entity.layer)) {
      throw new Error('Scene presentation layer is invalid.');
    }
    assertRecord(entity.poses, 'scene presentation poses');
    assertOnlyKeys(entity.poses, ['idle', 'attack', 'hit', 'defeated'], 'scene presentation poses');
    if (entity.poses.idle === undefined) throw new Error('Scene presentation has no idle pose.');
    for (const pose of ['idle', 'attack', 'hit', 'defeated']) {
      if (entity.poses[pose] !== undefined) {
        validateEntityVisualSource(entity.poses[pose], pose);
        poseCount += 1;
      }
    }
    if (poseCount > MAX_SCENE_PRESENTATION_POSES) {
      throw new Error('Scene presentation exceeds its pose bound.');
    }
  }
}

function validateEntityVisualSource(source, pose) {
  assertRecord(source, `scene presentation ${pose} source`);
  assertOnlyKeys(
    source,
    ['manifest', 'avif', 'webp', 'png'],
    `scene presentation ${pose} source`,
  );
  const supplied = Object.entries(source);
  if (supplied.length === 0 || supplied.length > MAX_ENTITY_SOURCE_REFERENCES) {
    throw new Error(`Scene presentation ${pose} source count is invalid.`);
  }
  for (const [kind, url] of supplied) {
    if (kind === 'manifest') {
      if (!isEntityManifestUrl(url)) {
        throw new Error(`Scene presentation ${pose} manifest is invalid.`);
      }
    } else if (!isEntityRasterUrl(url, pose, kind)) {
      throw new Error(`Scene presentation ${pose} ${kind} source is invalid.`);
    }
  }
}

function validateMessagePresentationCue(cue, actorId, targetId) {
  assertRecord(cue, 'message presentation cue');
  assertOnlyKeys(
    cue,
    ['kind', 'entityId', 'pose', 'intensity', 'durationMs'],
    'message presentation cue',
  );
  if (!['attack', 'hit', 'heal', 'defeat'].includes(cue.kind)) {
    throw new Error('Message presentation cue kind is invalid.');
  }
  if (cue.entityId !== undefined) {
    assertOpaqueId(cue.entityId, 'message presentation cue entity id');
    if (cue.entityId !== actorId && cue.entityId !== targetId) {
      throw new Error('Message presentation cue references an unrelated entity.');
    }
  }
  if (
    cue.pose !== undefined &&
    !['idle', 'attack', 'hit', 'defeated'].includes(cue.pose)
  ) {
    throw new Error('Message presentation cue pose is invalid.');
  }
  if (![1, 2, 3].includes(cue.intensity)) {
    throw new Error('Message presentation cue intensity is invalid.');
  }
  assertPositiveInteger(cue.durationMs, 'message presentation cue duration');
  if (cue.durationMs > 800) throw new Error('Message presentation cue duration exceeds 800 ms.');
}

function validateMediaChannel(channel, label) {
  if (!MEDIA_CHANNELS.has(channel)) throw new Error(`${label} is invalid.`);
}

function validateAudioUrl(url, label) {
  if (
    typeof url !== 'string' ||
    url.length > 161 ||
    !/^\/assets\/[a-f0-9]{64}\/audio-[a-z0-9-]+\.(?:opus|ogg|mp3)$/.test(url)
  ) {
    throw new Error(`${label} is invalid.`);
  }
}

function isEntityManifestUrl(value) {
  return typeof value === 'string' &&
    /^\/assets\/[a-f0-9]{64}\/manifest\.json$/.test(value);
}

function isEntityRasterUrl(value, pose, kind) {
  return typeof value === 'string' &&
    new RegExp(
      `^/assets/[a-f0-9]{64}/sprite-${pose}(?:-(?:320|640|1280))?\\.${kind}$`,
    ).test(value);
}

function validateVisualText(value, maximum, decorative, label) {
  if (
    typeof value !== 'string' ||
    value.length > maximum ||
    /[\u0000-\u001f\u007f]/.test(value) ||
    (decorative ? value !== '' : value.trim().length === 0)
  ) {
    throw new Error(`${label} is invalid.`);
  }
}

function validateSceneEntity(entity, kind, extraKeys = []) {
  assertRecord(entity, `scene ${kind}`);
  const itemKeys = kind === 'item'
    ? [
        'rarity', 'itemLevel', 'weightGrams', 'quantity', 'stackSlots',
        'totalWeightGrams', 'containerCapacity', 'transferPolicy', 'boundToYou', 'loot',
      ]
    : [];
  assertOnlyKeys(
    entity,
    ['id', 'label', 'description', 'actions', ...itemKeys, ...extraKeys],
    `scene ${kind}`,
  );
  assertOpaqueId(entity.id, `scene ${kind} id`);
  assertBoundedString(entity.label, 256, `scene ${kind} label`);
  assertStringMax(entity.description, 4096, `scene ${kind} description`);
  if (kind === 'item') {
    validateItemMetadata(entity, 'scene item');
    if (entity.loot !== undefined) validateSceneLootAccess(entity.loot, 'scene item loot');
  }
  assertArrayMax(entity.actions, 8, `scene ${kind} actions`);
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
  if (kind === 'item' && entity.loot?.access === 'reserved' && entity.actions.some((action) => {
    const command = action.command.trim().toLowerCase();
    return command.startsWith('take ') || command.startsWith('loot pass ');
  })) {
    throw new Error('Reserved scene loot exposed an unauthorized action.');
  }
}

function validatePlayerState(player) {
  assertRecord(player, 'player');
  assertOnlyKeys(
    player,
    [
      'revision', 'character', 'inventory', 'equipment', 'inventoryCapacity', 'map', 'quests', 'party',
      'partyInvites', 'economy', 'social', 'trade',
    ],
    'player',
  );
  assertPositiveInteger(player.revision, 'player revision');

  validatePlayerCharacter(player.character);

  validatePlayerInventory(
    player.inventory,
    player.equipment,
    player.inventoryCapacity,
    'player',
  );

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
  assertOnlyKeys(
    economy,
    ['coins', 'reservedCoins', 'spendableCoins', 'commodities', 'shopAvailable', 'shop'],
    'player economy',
  );
  assertNonNegativeInteger(economy.coins, 'player coins');
  assertNonNegativeInteger(economy.reservedCoins, 'player reserved coins');
  assertNonNegativeInteger(economy.spendableCoins, 'player spendable coins');
  if (economy.reservedCoins > economy.coins ||
      economy.spendableCoins !== economy.coins - economy.reservedCoins) {
    throw new Error('Player trade coin reservations are inconsistent.');
  }
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
    assertOnlyKeys(online, ['id', 'name', 'selfCharacter', 'nearby'], 'player online character');
    assertOpaqueId(online.id, 'player online character id');
    assertBoundedString(online.name, 64, 'player online character name');
    assertBoolean(online.selfCharacter, 'player online character self marker');
    assertBoolean(online.nearby, 'player online character proximity marker');
  }

  if (player.trade !== undefined && player.trade !== null) validateTrade(player.trade);

}

function validateTradeOffer(offer, label) {
  assertRecord(offer, label);
  assertOnlyKeys(offer, ['coins', 'lines'], label);
  assertNonNegativeInteger(offer.coins, `${label} coins`);
  if (offer.coins > 1_000_000_000) throw new Error(`${label} coins exceed the trade bound.`);
  assertArrayMax(offer.lines, 8, `${label} lines`);
  const targets = new Set();
  let roots = 0;
  for (const line of offer.lines) {
    assertRecord(line, `${label} line`);
    assertOnlyKeys(line, ['target', 'label', 'quantity', 'container'], `${label} line`);
    assertOpaqueId(line.target, `${label} line target`);
    assertBoundedString(line.label, 256, `${label} line label`);
    assertPositiveInteger(line.quantity, `${label} line quantity`);
    assertBoolean(line.container, `${label} line container marker`);
    if (line.quantity > 128 || (line.container && line.quantity !== 1) ||
        targets.has(line.target)) throw new Error(`${label} has an invalid line.`);
    targets.add(line.target);
    roots += line.quantity;
    if (roots > 128) throw new Error(`${label} exceeds the trade root bound.`);
  }
  return roots;
}

function validateTrade(trade) {
  assertRecord(trade, 'player trade');
  assertOnlyKeys(
    trade,
    [
      'id', 'phase', 'counterparty', 'expiresInTicks', 'stateRevision', 'ownConfirmed',
      'counterpartyConfirmed', 'ownOffer', 'counterpartyOffer', 'canAccept', 'canConfirm',
    ],
    'player trade',
  );
  assertOpaqueId(trade.id, 'player trade id');
  assertOpaqueId(trade.counterparty, 'player trade counterparty');
  if (!['requested', 'negotiating'].includes(trade.phase)) {
    throw new Error('Player trade phase is invalid.');
  }
  assertNonNegativeInteger(trade.expiresInTicks, 'player trade expiry');
  assertPositiveInteger(trade.stateRevision, 'player trade state revision');
  assertBoolean(trade.ownConfirmed, 'player trade own confirmation');
  assertBoolean(trade.counterpartyConfirmed, 'player trade counterparty confirmation');
  assertBoolean(trade.canAccept, 'player trade accept availability');
  assertBoolean(trade.canConfirm, 'player trade confirmation availability');
  const roots = validateTradeOffer(trade.ownOffer, 'player own trade offer') +
    validateTradeOffer(trade.counterpartyOffer, 'player counterparty trade offer');
  if (roots > 128) throw new Error('Player trade exceeds the shared root bound.');
  if (trade.phase === 'requested' &&
      (trade.ownOffer.coins !== 0 || trade.counterpartyOffer.coins !== 0 ||
        trade.ownOffer.lines.length > 0 || trade.counterpartyOffer.lines.length > 0 ||
        trade.ownConfirmed || trade.counterpartyConfirmed || trade.canConfirm)) {
    throw new Error('Requested trade has negotiating state.');
  }
  if (trade.phase === 'negotiating' && trade.canAccept) {
    throw new Error('Negotiating trade cannot be accepted again.');
  }
}

function validatePlayerCharacter(character) {
  assertRecord(character, 'player character');
  assertOnlyKeys(
    character,
    [
      'name', 'lineage', 'background', 'baseClass', 'activeClass', 'classHistory',
      'level', 'experience', 'nextLevelExperience', 'health', 'maxHealth',
      'strength', 'defence', 'attributes', 'skills', 'resources', 'abilities',
      'resistances', 'statuses', 'progressionPoints', 'talents', 'promotions',
      'effects', 'recoveryInTicks',
    ],
    'player character',
  );
  assertBoundedString(character.name, 64, 'player character name');
  assertPositiveInteger(character.level, 'player character level');
  if (character.level > 100) throw new Error('Player character level exceeds 100.');
  assertNonNegativeInteger(character.experience, 'player character experience');
  assertNonNegativeInteger(character.nextLevelExperience, 'player next level experience');
  assertNonNegativeInteger(character.health, 'player health');
  assertPositiveInteger(character.maxHealth, 'player max health');
  assertNonNegativeInteger(character.strength, 'player strength');
  assertNonNegativeInteger(character.defence, 'player defence');
  if (
    character.health > character.maxHealth ||
    character.maxHealth > 1_000_000 ||
    character.strength > 1_000_000 ||
    character.defence > 1_000_000
  ) {
    throw new Error('Player character vitals or attributes exceed their bounds.');
  }
  if (character.recoveryInTicks !== null && character.recoveryInTicks !== undefined) {
    assertNonNegativeInteger(character.recoveryInTicks, 'player recovery ticks');
  }

  const lineageId = validateProgressionOption(character.lineage, 'player lineage');
  const baseClassId = validateProgressionOption(character.baseClass, 'player base class');
  const activeClassId = validateProgressionOption(character.activeClass, 'player active class');
  if (character.background !== undefined) {
    validateProgressionOption(character.background, 'player background');
  }
  assertArrayMax(character.classHistory, 8, 'player class history');
  if (character.classHistory.length === 0) throw new Error('Player class history is empty.');
  const classIds = new Set();
  for (const entry of character.classHistory) {
    const id = validateProgressionOption(entry, 'player class history entry');
    if (classIds.has(id)) throw new Error('Player class history contains a duplicate class.');
    classIds.add(id);
  }
  if (!classIds.has(baseClassId) || !classIds.has(activeClassId)) {
    throw new Error('Player class history does not contain the base and active classes.');
  }
  if (!lineageId) throw new Error('Player lineage is invalid.');

  const attributeIds = new Set();
  assertArrayMax(character.attributes, 64, 'player attributes');
  for (const attribute of character.attributes) {
    assertRecord(attribute, 'player attribute');
    assertOnlyKeys(attribute, ['id', 'label', 'value'], 'player attribute');
    assertStatId(attribute.id, 'player attribute id');
    assertBoundedString(attribute.label, 128, 'player attribute label');
    assertSignedInteger(attribute.value, -2_147_483_648, 2_147_483_647, 'player attribute value');
    if (attributeIds.has(attribute.id)) throw new Error('Player attributes contain a duplicate id.');
    attributeIds.add(attribute.id);
  }

  const skillIds = new Set();
  assertArrayMax(character.skills, 64, 'player skills');
  for (const skill of character.skills) {
    assertRecord(skill, 'player skill');
    assertOnlyKeys(skill, ['id', 'label', 'rank'], 'player skill');
    assertStatId(skill.id, 'player skill id');
    assertBoundedString(skill.label, 128, 'player skill label');
    assertNonNegativeInteger(skill.rank, 'player skill rank');
    if (skill.rank > 1_000 || skillIds.has(skill.id)) {
      throw new Error('Player skills contain an invalid rank or duplicate id.');
    }
    skillIds.add(skill.id);
  }

  const resourceIds = new Set();
  assertArrayMax(character.resources, 16, 'player resources');
  for (const resource of character.resources) {
    assertRecord(resource, 'player resource');
    assertOnlyKeys(resource, ['id', 'label', 'current', 'max'], 'player resource');
    assertStatId(resource.id, 'player resource id');
    assertBoundedString(resource.label, 128, 'player resource label');
    assertNonNegativeInteger(resource.current, 'player resource current');
    assertPositiveInteger(resource.max, 'player resource maximum');
    if (resource.current > resource.max || resource.max > 1_000_000 || resourceIds.has(resource.id)) {
      throw new Error('Player resources contain an inconsistent value or duplicate id.');
    }
    resourceIds.add(resource.id);
  }

  const abilityIds = new Set();
  assertArrayMax(character.abilities, 64, 'player abilities');
  for (const ability of character.abilities) {
    assertRecord(ability, 'player ability');
    assertOnlyKeys(
      ability,
      [
        'id', 'name', 'description', 'kind', 'resourceCosts', 'cooldownRemaining',
        'usable', 'targeting',
      ],
      'player ability',
    );
    assertDefinitionId(ability.id, 'player ability id');
    assertBoundedString(ability.name, 128, 'player ability name');
    assertStringMax(ability.description, 2048, 'player ability description');
    if (!['martial', 'spell', 'utility'].includes(ability.kind)) {
      throw new Error('Player ability kind is invalid.');
    }
    assertNonNegativeInteger(ability.cooldownRemaining, 'player ability cooldown');
    if (ability.cooldownRemaining > 10_000) throw new Error('Player ability cooldown exceeds 10000.');
    assertBoolean(ability.usable, 'player ability usable state');
    if (!['self', 'optionalTarget', 'explicitTarget'].includes(ability.targeting)) {
      throw new Error('Player ability targeting is invalid.');
    }
    if (abilityIds.has(ability.id)) throw new Error('Player abilities contain a duplicate id.');
    abilityIds.add(ability.id);
    assertArrayMax(ability.resourceCosts, 4, 'player ability resource costs');
    const costResourceIds = new Set();
    for (const cost of ability.resourceCosts) {
      assertRecord(cost, 'player ability resource cost');
      assertOnlyKeys(cost, ['resourceId', 'label', 'amount'], 'player ability resource cost');
      assertStatId(cost.resourceId, 'player ability resource id');
      assertBoundedString(cost.label, 128, 'player ability resource label');
      assertPositiveInteger(cost.amount, 'player ability resource amount');
      if (
        cost.amount > 1_000_000 ||
        !resourceIds.has(cost.resourceId) ||
        costResourceIds.has(cost.resourceId)
      ) {
        throw new Error('Player ability resource cost is inconsistent.');
      }
      costResourceIds.add(cost.resourceId);
    }
  }

  assertArrayMax(character.resistances, 16, 'player resistances');
  const resistanceTypes = new Set();
  for (const resistance of character.resistances) {
    assertRecord(resistance, 'player resistance');
    assertOnlyKeys(resistance, ['damageType', 'percent'], 'player resistance');
    if (!['physical', 'arcane', 'fire', 'frost', 'lightning'].includes(resistance.damageType)) {
      throw new Error('Player resistance damage type is invalid.');
    }
    assertNonNegativeInteger(resistance.percent, 'player resistance percent');
    if (resistance.percent > 90 || resistanceTypes.has(resistance.damageType)) {
      throw new Error('Player resistances contain an invalid percent or duplicate type.');
    }
    resistanceTypes.add(resistance.damageType);
  }

  assertArrayMax(character.statuses, 32, 'player statuses');
  for (const status of character.statuses) validateStatus(status);

  assertRecord(character.progressionPoints, 'player progression points');
  const progressionPoints = Object.entries(character.progressionPoints);
  if (progressionPoints.length > 4) throw new Error('Player progression points exceed four pools.');
  for (const [poolId, amount] of progressionPoints) {
    assertStatId(poolId, 'player progression pool id');
    assertNonNegativeInteger(amount, 'player progression point amount');
    if (amount > 10_000) throw new Error('Player progression points exceed 10000.');
  }

  validateTalents(character.talents);
  validatePromotions(character.promotions);

  assertArrayMax(character.effects, 32, 'player effects');
  for (const effect of character.effects) {
    assertRecord(effect, 'player effect');
    assertOnlyKeys(effect, ['kind', 'magnitude', 'remainingTicks'], 'player effect');
    if (effect.kind !== 'regeneration' && effect.kind !== 'strength') {
      throw new Error('Player effect kind is invalid.');
    }
    assertPositiveInteger(effect.magnitude, 'player effect magnitude');
    assertPositiveInteger(effect.remainingTicks, 'player effect remaining ticks');
    if (effect.magnitude > 100_000 || effect.remainingTicks > 10_000) {
      throw new Error('Player effect exceeds its magnitude or duration bound.');
    }
  }
}

function validateProgressionOption(option, label) {
  assertRecord(option, label);
  assertOnlyKeys(option, ['id', 'label', 'description'], label);
  assertDefinitionId(option.id, `${label} id`);
  assertBoundedString(option.label, 128, `${label} label`);
  assertStringMax(option.description, 2048, `${label} description`);
  return option.id;
}

function validateRequirementStatuses(requirements, label) {
  assertArrayMax(requirements, 128, label);
  for (const requirement of requirements) {
    assertRecord(requirement, `${label} entry`);
    assertOnlyKeys(requirement, ['met', 'text'], `${label} entry`);
    assertBoolean(requirement.met, `${label} met state`);
    assertBoundedString(requirement.text, 512, `${label} text`);
  }
}

function validateStatus(status) {
  assertRecord(status, 'player status');
  assertOnlyKeys(status, ['id', 'label', 'kind', 'magnitude', 'remainingTicks'], 'player status');
  assertDefinitionId(status.id, 'player status id');
  assertBoundedString(status.label, 128, 'player status label');
  if (!['attribute_modifier', 'regeneration', 'damage_over_time', 'stun'].includes(status.kind)) {
    throw new Error('Player status kind is invalid.');
  }
  assertSignedInteger(status.magnitude, -100_000, 100_000, 'player status magnitude');
  assertPositiveInteger(status.remainingTicks, 'player status remaining ticks');
  if (status.remainingTicks > 10_000) throw new Error('Player status duration exceeds 10000.');
  if (
    (status.kind === 'attribute_modifier' && status.magnitude === 0) ||
    (['regeneration', 'damage_over_time'].includes(status.kind) && status.magnitude <= 0) ||
    (status.kind === 'stun' && status.magnitude !== 0)
  ) {
    throw new Error('Player status magnitude is inconsistent with its kind.');
  }
}

function validateTalents(talents) {
  assertArrayMax(talents, 16, 'player talent graphs');
  const graphIds = new Set();
  for (const graph of talents) {
    assertRecord(graph, 'player talent graph');
    assertOnlyKeys(
      graph,
      ['id', 'label', 'description', 'pointPoolId', 'availablePoints', 'nodes'],
      'player talent graph',
    );
    assertDefinitionId(graph.id, 'player talent graph id');
    assertBoundedString(graph.label, 128, 'player talent graph label');
    assertStringMax(graph.description, 2048, 'player talent graph description');
    assertStatId(graph.pointPoolId, 'player talent point pool id');
    assertNonNegativeInteger(graph.availablePoints, 'player talent available points');
    if (graph.availablePoints > 10_000 || graphIds.has(graph.id)) {
      throw new Error('Player talent graph has invalid points or a duplicate id.');
    }
    graphIds.add(graph.id);
    assertArrayMax(graph.nodes, 32, 'player talent nodes');
    const nodeIds = new Set();
    for (const node of graph.nodes) {
      assertRecord(node, 'player talent node');
      assertOnlyKeys(
        node,
        [
          'id', 'label', 'description', 'rank', 'maxRank', 'nextCost', 'available',
          'requirements', 'command',
        ],
        'player talent node',
      );
      assertDefinitionId(node.id, 'player talent node id');
      assertBoundedString(node.label, 128, 'player talent node label');
      assertStringMax(node.description, 2048, 'player talent node description');
      assertNonNegativeInteger(node.rank, 'player talent node rank');
      assertPositiveInteger(node.maxRank, 'player talent node max rank');
      if (node.maxRank > 5 || node.rank > node.maxRank || nodeIds.has(node.id)) {
        throw new Error('Player talent node has an invalid rank or duplicate id.');
      }
      if (node.nextCost !== null && node.nextCost !== undefined) {
        assertPositiveInteger(node.nextCost, 'player talent node next cost');
        if (node.nextCost > 10_000 || node.rank === node.maxRank) {
          throw new Error('Player talent node next cost is inconsistent.');
        }
      }
      assertBoolean(node.available, 'player talent node availability');
      validateRequirementStatuses(node.requirements, 'player talent node requirements');
      assertBoundedString(node.command, 1024, 'player talent node command');
      nodeIds.add(node.id);
    }
  }
}

function validatePromotions(promotions) {
  assertArrayMax(promotions, 64, 'player promotions');
  const classIds = new Set();
  for (const promotion of promotions) {
    assertRecord(promotion, 'player promotion');
    assertOnlyKeys(promotion, ['class', 'available', 'requirements', 'command'], 'player promotion');
    const classId = validateProgressionOption(promotion.class, 'player promotion class');
    if (classIds.has(classId)) throw new Error('Player promotions contain a duplicate class.');
    classIds.add(classId);
    assertBoolean(promotion.available, 'player promotion availability');
    validateRequirementStatuses(promotion.requirements, 'player promotion requirements');
    assertBoundedString(promotion.command, 1024, 'player promotion command');
  }
}

function validatePlayerInventory(inventory, equipment, capacity, label) {
  assertArrayMax(inventory, 256, `${label} inventory`);
  assertArrayMax(equipment, 3, `${label} equipment`);
  const instanceIds = new Set();
  for (const item of inventory) {
    const itemLabel = `${label} inventory item`;
    assertRecord(item, itemLabel);
    assertOnlyKeys(
      item,
      [
        'id', 'label', 'description', 'rarity', 'itemLevel', 'weightGrams', 'quantity',
        'availableQuantity', 'reservedForTrade', 'stackSlots', 'totalWeightGrams',
        'containerCapacity', 'transferPolicy', 'boundToYou', 'canGive', 'canTrade', 'actions',
      ],
      itemLabel,
    );
    assertOpaqueId(item.id, `${itemLabel} id`);
    assertBoundedString(item.label, 256, `${itemLabel} label`);
    assertStringMax(item.description, 4096, `${itemLabel} description`);
    validateItemMetadata(item, itemLabel);
    if (item.transferPolicy === 'character_bound' && !item.boundToYou) {
      throw new Error(`${itemLabel} is character-bound but not bound to this character.`);
    }
    assertBoolean(item.canGive, `${itemLabel} give availability`);
    if (item.canGive && item.transferPolicy !== 'transferable') {
      throw new Error(`${itemLabel} cannot be given under its transfer policy.`);
    }
    validateItemTradeAvailability(item, itemLabel);
    validateActions(item.actions, `${itemLabel} actions`);
    if (instanceIds.has(item.id)) throw new Error(`${label} inventory contains a duplicate item id.`);
    instanceIds.add(item.id);
  }

  const occupiedSlots = new Set();
  for (const item of equipment) {
    const itemLabel = `${label} equipment item`;
    assertRecord(item, itemLabel);
    assertOnlyKeys(
      item,
      [
        'id', 'slot', 'label', 'rarity', 'itemLevel', 'weightGrams', 'quantity',
        'stackSlots', 'totalWeightGrams', 'containerCapacity', 'transferPolicy', 'boundToYou',
        'actions',
      ],
      itemLabel,
    );
    assertOpaqueId(item.id, `${itemLabel} id`);
    if (!EQUIPMENT_SLOTS.has(item.slot)) throw new Error(`${itemLabel} slot is invalid.`);
    assertBoundedString(item.label, 256, `${itemLabel} label`);
    validateItemMetadata(item, itemLabel);
    if (item.transferPolicy === 'character_bound' && !item.boundToYou) {
      throw new Error(`${itemLabel} is character-bound but not bound to this character.`);
    }
    validateActions(item.actions, `${itemLabel} actions`);
    if (instanceIds.has(item.id)) throw new Error(`${label} contains a duplicate item id.`);
    if (occupiedSlots.has(item.slot)) throw new Error(`${label} equipment contains a duplicate slot.`);
    instanceIds.add(item.id);
    occupiedSlots.add(item.slot);
  }

  validateInventoryCapacity(capacity, inventory, equipment, label);
}

function validateItemTradeAvailability(item, label) {
  assertNonNegativeInteger(item.availableQuantity, `${label} available quantity`);
  assertNonNegativeInteger(item.reservedForTrade, `${label} reserved quantity`);
  assertBoolean(item.canTrade, `${label} trade availability`);
  if (item.availableQuantity + item.reservedForTrade !== item.quantity ||
      (item.canTrade &&
        (item.availableQuantity === 0 || item.transferPolicy !== 'transferable'))) {
    throw new Error(`${label} has inconsistent trade availability.`);
  }
}

function validateItemMetadata(item, label) {
  if (!ITEM_RARITIES.has(item.rarity)) throw new Error(`${label} rarity is invalid.`);
  if (!['transferable', 'character_bound', 'non_transferable'].includes(item.transferPolicy)) {
    throw new Error(`${label} transferPolicy is invalid.`);
  }
  assertBoolean(item.boundToYou, `${label} binding marker`);
  if (item.transferPolicy !== 'character_bound' && item.boundToYou) {
    throw new Error(`${label} has an invalid binding marker.`);
  }
  assertPositiveInteger(item.itemLevel, `${label} itemLevel`);
  if (item.itemLevel > MAX_ITEM_LEVEL) {
    throw new Error(`${label} itemLevel exceeds ${MAX_ITEM_LEVEL}.`);
  }
  assertNonNegativeInteger(item.weightGrams, `${label} weightGrams`);
  if (item.weightGrams > MAX_ITEM_WEIGHT_GRAMS) {
    throw new Error(`${label} weightGrams exceeds ${MAX_ITEM_WEIGHT_GRAMS}.`);
  }
  assertPositiveInteger(item.quantity, `${label} quantity`);
  if (item.quantity > MAX_ITEM_STACK_UNITS) throw new Error(`${label} quantity is too large.`);
  assertPositiveInteger(item.stackSlots, `${label} stackSlots`);
  if (item.stackSlots > MAX_ITEM_SLOT_COST) throw new Error(`${label} stackSlots is too large.`);
  assertNonNegativeInteger(item.totalWeightGrams, `${label} totalWeightGrams`);
  if (item.totalWeightGrams > MAX_ITEM_TOTAL_WEIGHT_GRAMS) {
    throw new Error(`${label} totalWeightGrams is too large.`);
  }
  const unitTotal = item.weightGrams * item.quantity;
  if (item.containerCapacity === undefined) {
    if (item.totalWeightGrams !== unitTotal) throw new Error(`${label} total weight is inconsistent.`);
  } else {
    if (
      item.quantity !== 1 ||
      item.totalWeightGrams !== unitTotal + item.containerCapacity.usedWeightGrams
    ) {
      throw new Error(`${label} container total weight is inconsistent.`);
    }
    validateCapacityShape(item.containerCapacity, `${label} container capacity`);
  }
}

function validateSceneLootAccess(loot, label) {
  assertRecord(loot, label);
  if (loot.access === 'reserved') {
    assertOnlyKeys(loot, ['access'], label);
    return;
  }
  if (loot.access !== 'yours') throw new Error(`${label} access is invalid.`);
  assertOnlyKeys(loot, ['access', 'expiresInTicks'], label);
  assertPositiveInteger(loot.expiresInTicks, `${label} expiry`);
  if (loot.expiresInTicks > 20) throw new Error(`${label} expiry exceeds 20 ticks.`);
}

function validateInventoryCapacity(capacity, inventory, equipment, label) {
  validateCapacityShape(capacity, `${label} inventory capacity`);
  const projectedWeight = [...inventory, ...equipment]
    .reduce((total, item) => total + item.totalWeightGrams, 0);
  if (capacity.usedWeightGrams !== projectedWeight) {
    throw new Error(`${label} used weight does not match its projected items.`);
  }
  const projectedSlots = inventory.reduce((total, item) => total + item.stackSlots, 0);
  if (capacity.usedSlots !== projectedSlots) {
    throw new Error(`${label} used slots do not match its projected inventory.`);
  }
}

function validateCapacityShape(capacity, label) {
  assertRecord(capacity, label);
  assertOnlyKeys(
    capacity,
    ['usedSlots', 'maxSlots', 'usedWeightGrams', 'maxWeightGrams'],
    label,
  );
  assertNonNegativeInteger(capacity.usedSlots, `${label} used slots`);
  assertPositiveInteger(capacity.maxSlots, `${label} max slots`);
  assertNonNegativeInteger(capacity.usedWeightGrams, `${label} used weight`);
  assertNonNegativeInteger(capacity.maxWeightGrams, `${label} max weight`);
  if (capacity.maxSlots > MAX_INVENTORY_SLOTS || capacity.usedSlots > capacity.maxSlots) {
    throw new Error(`${label} inventory slot capacity is inconsistent.`);
  }
  if (
    capacity.maxWeightGrams > MAX_INVENTORY_WEIGHT_GRAMS ||
    capacity.usedWeightGrams > capacity.maxWeightGrams
  ) {
    throw new Error(`${label} inventory weight capacity is inconsistent.`);
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
    ['revision', 'kind', 'text', 'actorId', 'targetId', 'itemId', 'cue'],
    'message.append.data',
  );
  assertPositiveInteger(event.revision, 'message revision');
  if (![
    'characterMoved',
    'itemTaken',
    'itemDropped',
    'itemGiven',
    'tradeRequested',
    'tradeChanged',
    'tradeCompleted',
    'tradeCancelled',
    'gameplay',
    'adventure',
  ].includes(event.kind)) {
    throw new Error(`Unknown message.append kind ${event.kind}.`);
  }
  requiredString(event.text, 'message text');
  assertOpaqueId(event.actorId, 'message actorId');
  if (event.targetId !== undefined) assertOpaqueId(event.targetId, 'message targetId');
  if (event.itemId !== undefined) assertOpaqueId(event.itemId, 'message itemId');
  if (event.cue !== undefined) {
    validateMessagePresentationCue(event.cue, event.actorId, event.targetId);
  }
}

function requiredExit(scene, command) {
  const exit = scene.exits.find((candidate) => candidate.command === command);
  if (!exit) throw new Error(`Scene ${scene.title} has no authorized ${command} exit.`);
  assertOpaqueId(exit.id, `${command} exit id`);
  return exit;
}

function optionalExit(scene, command) {
  const exit = scene.exits.find((candidate) => candidate.command === command);
  if (exit) assertOpaqueId(exit.id, `${command} exit id`);
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

function validateSyncId(value, label) {
  if (typeof value !== 'string' || !/^sync-[1-9][0-9]{0,19}$/.test(value)) {
    throw new Error(`${label} is invalid.`);
  }
}

function assertContentId(value, label) {
  if (typeof value !== 'string' || !/^[a-z0-9][a-z0-9-]{0,63}$/.test(value)) {
    throw new Error(`${label} is not a bounded content id.`);
  }
}

function assertDefinitionId(value, label) {
  if (
    typeof value !== 'string' ||
    !/^[a-z0-9](?:[a-z0-9_.:-]{0,126}[a-z0-9])?$/.test(value)
  ) {
    throw new Error(`${label} is not a bounded definition id.`);
  }
}

function assertStatId(value, label) {
  if (
    typeof value !== 'string' ||
    !/^[a-z0-9](?:[a-z0-9_.:-]{0,62}[a-z0-9])?$/.test(value)
  ) {
    throw new Error(`${label} is not a bounded stat id.`);
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

function assertSignedInteger(value, minimum, maximum, label) {
  if (!Number.isSafeInteger(value) || value < minimum || value > maximum) {
    throw new Error(`${label} is outside [${minimum}, ${maximum}].`);
  }
}

function assertFiniteRange(value, minimum, maximum, label) {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < minimum || value > maximum) {
    throw new Error(`${label} is outside [${minimum}, ${maximum}].`);
  }
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
