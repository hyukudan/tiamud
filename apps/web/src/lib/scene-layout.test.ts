import { describe, expect, it } from 'vitest';
import { layoutSceneEntities, type VisualLayoutEntity } from './scene-layout';

const entity = (
  id: string,
  priority = 0,
  width = 0.16,
  height = 0.35,
): VisualLayoutEntity => ({ id, priority, footprint: { width, height } });

describe('scene entity layout', () => {
  it('is deterministic, priority ordered and non-overlapping for a normal formation', () => {
    const input = [entity('item:lantern', 1, 0.1, 0.16), entity('npc:wolf', 10), entity('npc:guard', 8)];
    const first = layoutSceneEntities(input);
    const second = layoutSceneEntities(input.toReversed());

    expect(first).toEqual(second);
    expect(first.placements.map(({ id }) => id)).toEqual(['npc:wolf', 'npc:guard', 'item:lantern']);
    expect(first.overflow).toEqual([]);
    for (const [index, placement] of first.placements.entries()) {
      for (const other of first.placements.slice(index + 1)) {
        const horizontal = placement.rect.x + placement.rect.width <= other.rect.x
          || other.rect.x + other.rect.width <= placement.rect.x;
        const vertical = placement.rect.y + placement.rect.height <= other.rect.y
          || other.rect.y + other.rect.height <= placement.rect.y;
        expect(horizontal || vertical).toBe(true);
      }
    }
  });

  it('uses opaque ID order as the stable tie-breaker with no arrival-history state', () => {
    const snapshot = [entity('o9999999999999999', 10), entity('o1111111111111111', 10)];
    const initial = layoutSceneEntities(snapshot);
    const reconnect = layoutSceneEntities(snapshot.toReversed());
    expect(initial).toEqual(reconnect);
    expect(initial.placements.map(({ id }) => id)).toEqual([
      'o1111111111111111',
      'o9999999999999999',
    ]);
  });

  it('keeps authored safe zones clear and uses overflow when no placement fits', () => {
    const fullStage = [{ x: 0, y: 0, width: 1, height: 1 }];
    const result = layoutSceneEntities([entity('npc:wolf')], { safeZones: fullStage });
    expect(result.placements).toEqual([]);
    expect(result.overflow).toEqual(['npc:wolf']);
  });

  it('bounds the visible DOM and sends lower-priority entities to overflow', () => {
    const result = layoutSceneEntities(
      [entity('npc:boss', 100), entity('npc:second', 10), entity('item:coin', 0)],
      { maxVisible: 2 },
    );
    expect(result.placements.map(({ id }) => id)).toEqual(['npc:boss', 'npc:second']);
    expect(result.overflow).toEqual(['item:coin']);
  });

  it('deduplicates entity IDs before layout', () => {
    const result = layoutSceneEntities([entity('npc:wolf', 1), entity('npc:wolf', 99)]);
    expect(result.placements).toHaveLength(1);
    expect(result.overflow).toEqual([]);
  });
});
