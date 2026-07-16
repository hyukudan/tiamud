export interface NormalizedPoint {
  x: number;
  y: number;
}

export interface NormalizedSize {
  width: number;
  height: number;
}

export interface NormalizedRect extends NormalizedPoint, NormalizedSize {}

export interface VisualLayoutEntity {
  id: string;
  priority: number;
  footprint: NormalizedSize;
  preferredAnchor?: NormalizedPoint;
}

export interface VisualPlacement {
  id: string;
  slot: string;
  scale: number;
  anchor: NormalizedPoint;
  rect: NormalizedRect;
}

export interface SceneLayoutOptions {
  safeZones?: NormalizedRect[];
  maxVisible?: number;
  minimumScale?: number;
  overlapTolerance?: number;
}

export interface SceneLayoutResult {
  placements: VisualPlacement[];
  overflow: string[];
}

interface CandidateSlot {
  id: string;
  anchor: NormalizedPoint;
}

const DEFAULT_MAX_VISIBLE = 10;
const DEFAULT_MINIMUM_SCALE = 0.7;
const EPSILON = 1e-9;

// Anchors describe the bottom-centre of a transparent entity asset. The back
// row is considered only after the foreground row, preserving a predictable
// RPG-like formation without tying WorldPack data to CSS pixels.
const DEFAULT_SLOTS: CandidateSlot[] = [
  { id: 'front-centre', anchor: { x: 0.5, y: 0.96 } },
  { id: 'front-left', anchor: { x: 0.3, y: 0.96 } },
  { id: 'front-right', anchor: { x: 0.7, y: 0.96 } },
  { id: 'front-far-left', anchor: { x: 0.11, y: 0.96 } },
  { id: 'front-far-right', anchor: { x: 0.89, y: 0.96 } },
  { id: 'back-centre', anchor: { x: 0.5, y: 0.72 } },
  { id: 'back-left', anchor: { x: 0.3, y: 0.72 } },
  { id: 'back-right', anchor: { x: 0.7, y: 0.72 } },
  { id: 'back-far-left', anchor: { x: 0.11, y: 0.72 } },
  { id: 'back-far-right', anchor: { x: 0.89, y: 0.72 } },
];

function clamp(value: number, minimum = 0, maximum = 1): number {
  return Math.min(maximum, Math.max(minimum, Number.isFinite(value) ? value : minimum));
}

function boundedSize(size: NormalizedSize): NormalizedSize {
  return {
    width: clamp(size.width, 0.02, 1),
    height: clamp(size.height, 0.02, 1),
  };
}

function rectAt(anchor: NormalizedPoint, footprint: NormalizedSize, scale: number): NormalizedRect {
  const width = footprint.width * scale;
  const height = footprint.height * scale;
  const x = clamp(anchor.x - width / 2, 0, Math.max(0, 1 - width));
  const y = clamp(anchor.y - height, 0, Math.max(0, 1 - height));
  return { x, y, width, height };
}

function overlapArea(left: NormalizedRect, right: NormalizedRect): number {
  const width = Math.max(0, Math.min(left.x + left.width, right.x + right.width) - Math.max(left.x, right.x));
  const height = Math.max(0, Math.min(left.y + left.height, right.y + right.height) - Math.max(left.y, right.y));
  return width * height;
}

function distanceSquared(left: NormalizedPoint, right: NormalizedPoint): number {
  const x = left.x - right.x;
  const y = left.y - right.y;
  return x * x + y * y;
}

function orderedCandidates(entity: VisualLayoutEntity): CandidateSlot[] {
  const preferred = entity.preferredAnchor
    ? { x: clamp(entity.preferredAnchor.x), y: clamp(entity.preferredAnchor.y) }
    : { x: 0.5, y: 0.96 };
  return DEFAULT_SLOTS.toSorted((left, right) => {
    const distance = distanceSquared(left.anchor, preferred) - distanceSquared(right.anchor, preferred);
    return Math.abs(distance) > EPSILON ? distance : left.id.localeCompare(right.id);
  });
}

function scales(minimum: number): number[] {
  const values = [1, 0.85, minimum]
    .map((value) => clamp(value, minimum, 1));
  return [...new Set(values)];
}

/**
 * Places presentation-only scene entities in deterministic normalized slots.
 *
 * The function deliberately prefers overflow over material overlap. Its result
 * is safe to recompute for any viewport and never influences authoritative
 * visibility, targeting, combat or text output.
 */
export function layoutSceneEntities(
  source: readonly VisualLayoutEntity[],
  options: SceneLayoutOptions = {},
): SceneLayoutResult {
  const maximum = Math.max(0, Math.floor(options.maxVisible ?? DEFAULT_MAX_VISIBLE));
  const minimumScale = clamp(options.minimumScale ?? DEFAULT_MINIMUM_SCALE, 0.4, 1);
  const tolerance = Math.max(0, options.overlapTolerance ?? 0);
  const safeZones = (options.safeZones ?? []).map((zone) => ({
    x: clamp(zone.x),
    y: clamp(zone.y),
    width: clamp(zone.width, 0, 1),
    height: clamp(zone.height, 0, 1),
  }));
  const entities = source
    .filter((entity) => entity.id.length > 0)
    .map((entity) => ({ ...entity, footprint: boundedSize(entity.footprint) }))
    .toSorted((left, right) => right.priority - left.priority || left.id.localeCompare(right.id))
    .filter((entity, index, values) => values.findIndex((candidate) => candidate.id === entity.id) === index);

  const placements: VisualPlacement[] = [];
  const overflow: string[] = [];
  for (const entity of entities) {
    if (placements.length >= maximum) {
      overflow.push(entity.id);
      continue;
    }
    let selected: VisualPlacement | undefined;
    for (const scale of scales(minimumScale)) {
      for (const candidate of orderedCandidates(entity)) {
        const rect = rectAt(candidate.anchor, entity.footprint, scale);
        const collides = placements.some((placement) => overlapArea(rect, placement.rect) > tolerance)
          || safeZones.some((zone) => overlapArea(rect, zone) > tolerance);
        if (!collides) {
          selected = {
            id: entity.id,
            slot: candidate.id,
            scale,
            anchor: candidate.anchor,
            rect,
          };
          break;
        }
      }
      if (selected) break;
    }
    if (selected) placements.push(selected);
    else overflow.push(entity.id);
  }
  return { placements, overflow };
}
