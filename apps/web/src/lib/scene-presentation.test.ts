import { describe, expect, it } from 'vitest';
import type { MessagePresentationCue, SceneEntityPresentation, SceneSnapshot } from './protocol';
import {
  mapVisibleScenePresentation,
  presentationCueEntityId,
  presentationCuePose,
  reconcileScenePresentationCues,
  visualSourceForCue,
  type ActiveScenePresentationCue,
} from './scene-presentation';

const hash = 'a'.repeat(64);
const annotation = (id: string): SceneEntityPresentation => ({
  id,
  alt: '',
  decorative: true,
  footprint: { width: 0.2, height: 0.4 },
  priority: 10,
  layer: 'foreground',
  poses: {
    idle: { png: `/assets/${hash}/sprite-idle-640.png` },
    attack: { webp: `/assets/${hash}/sprite-attack-640.webp` },
    hit: { png: `/assets/${hash}/sprite-hit-640.png` },
    defeated: { webp: `/assets/${hash}/sprite-defeated-640.webp` },
  },
});

const scene = (entities: SceneEntityPresentation[]): SceneSnapshot => ({
  viewId: 'o9999999999999999',
  revision: 1,
  title: 'Lantern Square',
  prose: ['Text remains authoritative.'],
  exits: [],
  items: [{
    id: 'o1111111111111111', label: 'lantern', description: 'Cold light.',
    rarity: 'uncommon', itemLevel: 3, weightGrams: 800,
    quantity: 1, stackSlots: 1, totalWeightGrams: 800, actions: [],
  }],
  npcs: [{
    id: 'o2222222222222222', label: 'rain wolf', description: 'It watches.',
    health: 10, maxHealth: 10, actions: [],
  }],
  weather: { condition: 'rain', day: 1, hour: 20, nextChangeInTicks: 3 },
  presentation: { entities },
});

describe('scene presentation sidecar', () => {
  it('maps annotations only to semantic entities visible in the same snapshot', () => {
    const result = mapVisibleScenePresentation(scene([
      annotation('o2222222222222222'),
      annotation('o3333333333333333'),
      annotation('o1111111111111111'),
    ]));

    expect(result.map(({ id, label }) => ({ id, label }))).toEqual([
      { id: 'o2222222222222222', label: 'rain wolf' },
      { id: 'o1111111111111111', label: 'lantern' },
    ]);
  });

  it('cannot create a visual entity without a semantic scene entity', () => {
    const hiddenOnly = scene([annotation('o3333333333333333')]);
    hiddenOnly.items = [];
    hiddenOnly.npcs = [];
    expect(mapVisibleScenePresentation(hiddenOnly)).toEqual([]);
  });

  it('selects cue subjects deterministically from the same semantic message', () => {
    const message = {
      revision: 2,
      kind: 'gameplay' as const,
      text: 'The wolf strikes the guard.',
      actorId: 'o1111111111111111',
      targetId: 'o2222222222222222',
    };
    expect(presentationCueEntityId(
      { kind: 'attack', intensity: 1, durationMs: 300 },
      message,
    )).toBe(message.actorId);
    for (const kind of ['hit', 'heal', 'defeat'] as const) {
      expect(presentationCueEntityId(
        { kind, intensity: 1, durationMs: 300 },
        message,
      )).toBe(message.targetId);
    }
    expect(presentationCueEntityId(
      { kind: 'hit', entityId: message.actorId, intensity: 1, durationMs: 300 },
      message,
    )).toBe(message.actorId);
  });

  it('selects authored attack, hit and defeated poses without changing them for reduced motion', () => {
    const entity = annotation('o2222222222222222');
    const cues = [
      [{ kind: 'attack', intensity: 1, durationMs: 300 }, 'attack', 'sprite-attack-640.webp'],
      [{ kind: 'hit', intensity: 2, durationMs: 300 }, 'hit', 'sprite-hit-640.png'],
      [{ kind: 'defeat', intensity: 3, durationMs: 600 }, 'defeated', 'sprite-defeated-640.webp'],
    ] as const satisfies readonly [MessagePresentationCue, string, string][];

    for (const [cue, pose, file] of cues) {
      expect(presentationCuePose(cue)).toBe(pose);
      expect(JSON.stringify(visualSourceForCue(entity, cue))).toContain(file);
    }
    // Motion policy changes CSS animation only; it never changes or fetches a
    // different pose URL.
    expect(visualSourceForCue(entity, cues[1][0])).toEqual(entity.poses.hit);
  });

  it('retains a cue across the immediate same-view snapshot and clears it on removal or view change', () => {
    const before = scene([annotation('o2222222222222222')]);
    const immediate = scene([annotation('o2222222222222222')]);
    immediate.revision = 2;
    for (const [kind, pose] of [
      ['attack', 'attack'],
      ['hit', 'hit'],
      ['defeat', 'defeated'],
    ] as const) {
      const active: Record<string, ActiveScenePresentationCue> = {
        o2222222222222222: {
          kind,
          intensity: 2,
          durationMs: 400,
          pose,
          eventKey: `11:${kind}:2`,
        },
      };
      expect(reconcileScenePresentationCues(before, immediate, active)).toEqual(active);
    }

    const active: Record<string, ActiveScenePresentationCue> = {
      o2222222222222222: {
        kind: 'defeat', intensity: 2, durationMs: 400, pose: 'defeated', eventKey: '11:defeat:2',
      },
    };

    const removed = scene([]);
    removed.revision = 3;
    expect(reconcileScenePresentationCues(immediate, removed, active)).toEqual({});

    const changedView = scene([annotation('o2222222222222222')]);
    changedView.viewId = 'o8888888888888888';
    expect(reconcileScenePresentationCues(immediate, changedView, active)).toEqual({});
  });

  it('never selects a source for an orphan presentation annotation', () => {
    const orphan = annotation('o3333333333333333');
    const visible = mapVisibleScenePresentation(scene([orphan]));
    expect(visible.find(({ id }) => id === orphan.id)).toBeUndefined();
    expect(visible.flatMap(({ visual }) => visualSourceForCue(
      visual,
      { kind: 'attack', intensity: 1, durationMs: 300 },
    ) ?? [])).toEqual([]);
  });
});
