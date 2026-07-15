import { describe, expect, it } from 'vitest';
import { createClientHello, createCommandSubmit, parseServerMessage } from './protocol';
import { websocketUrl } from './socket';

describe('protocol messages', () => {
  it('announces modern image formats with AVIF first', () => {
    expect(createClientHello(true).data.capabilities).toEqual({
      images: ['avif', 'webp', 'jpeg'],
      reducedMotion: true,
    });
  });

  it('creates commands with an idempotency identifier', () => {
    expect(createCommandSubmit('north', 'request-1')).toMatchObject({
      type: 'command.submit',
      data: { command: 'north', requestId: 'request-1' },
    });
  });

  it('normalises the current visual scene shape', () => {
    const message = parseServerMessage(
      JSON.stringify({
        v: 1,
        type: 'scene.snapshot',
        seq: 8,
        data: {
          title: 'The old road',
          description: 'Rain softens the outlines of the forest.',
          visual: {
            alt: 'A forest road in light rain.',
            variants: [
              { format: 'avif', url: '/assets/road.avif' },
              { format: 'webp', url: '/assets/road.webp' },
              { format: 'jpeg', url: '/assets/road.jpg' },
            ],
          },
          exits: [{ id: 'north', label: 'North', command: 'north' }],
        },
      }),
    );

    expect(message).toMatchObject({
      type: 'scene.snapshot',
      data: {
        title: 'The old road',
        prose: ['Rain softens the outlines of the forest.'],
        image: {
          avif: '/assets/road.avif',
          webp: '/assets/road.webp',
          jpeg: '/assets/road.jpg',
        },
        exits: [{ id: 'north', label: 'North', command: 'north' }],
      },
    });
  });

  it('rejects malformed or unknown messages', () => {
    expect(parseServerMessage('{')).toBeUndefined();
    expect(parseServerMessage('{"type":"world.destroyed","data":{}}')).toBeUndefined();
  });
});

describe('websocket URL', () => {
  it('uses secure WebSockets on HTTPS', () => {
    expect(websocketUrl({ protocol: 'https:', host: 'mud.example' })).toBe(
      'wss://mud.example/ws',
    );
  });

  it('uses local WebSockets on HTTP', () => {
    expect(websocketUrl({ protocol: 'http:', host: 'localhost:5173' })).toBe(
      'ws://localhost:5173/ws',
    );
  });
});
