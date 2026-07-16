import type {
  MessageAppendData,
  MessagePresentationCue,
  SceneEntityPresentation,
  SceneEntityPose,
  SceneEntityVisualSource,
  SceneSnapshot,
} from './protocol';

export interface VisibleScenePresentation {
  id: string;
  label: string;
  visual: SceneEntityPresentation;
}

/**
 * Joins presentation-only annotations to the authoritative visible snapshot.
 * An annotation can decorate an entity, but can never create or reveal one.
 */
export function mapVisibleScenePresentation(scene: SceneSnapshot | undefined): VisibleScenePresentation[] {
  const visible = new Map([
    ...(scene?.npcs ?? []),
    ...(scene?.items ?? []),
  ].map((entity) => [entity.id, entity] as const));

  return (scene?.presentation?.entities ?? []).flatMap((annotation) => {
    const entity = visible.get(annotation.id);
    return entity ? [{ id: entity.id, label: entity.label, visual: annotation }] : [];
  });
}

/** Selects the visual subject from the same authoritative semantic event. */
export function presentationCueEntityId(
  cue: MessagePresentationCue,
  message: MessageAppendData,
): string {
  if (cue.entityId) return cue.entityId;
  return cue.kind === 'attack' ? message.actorId : (message.targetId ?? message.actorId);
}

export interface ActiveScenePresentationCue extends MessagePresentationCue {
  pose: SceneEntityPose;
  eventKey: string;
}

export function presentationCuePose(cue: MessagePresentationCue): SceneEntityPose {
  if (cue.pose) return cue.pose;
  if (cue.kind === 'attack') return 'attack';
  if (cue.kind === 'hit') return 'hit';
  if (cue.kind === 'defeat') return 'defeated';
  return 'idle';
}

/**
 * A normal same-view snapshot follows each semantic message. Keep its bounded
 * cue only while the opaque subject remains in the new authoritative scene.
 * Recovery and view changes deliberately use a separate full reset.
 */
export function reconcileScenePresentationCues(
  previous: SceneSnapshot | undefined,
  next: SceneSnapshot,
  active: Readonly<Record<string, ActiveScenePresentationCue>>,
): Record<string, ActiveScenePresentationCue> {
  if (!previous?.viewId || previous.viewId !== next.viewId) return {};
  const visible = new Set(mapVisibleScenePresentation(next).map(({ id }) => id));
  return Object.fromEntries(
    Object.entries(active).filter(([entityId]) => visible.has(entityId)),
  );
}

export function visualSourceForCue(
  entity: SceneEntityPresentation,
  cue: MessagePresentationCue | undefined,
): SceneEntityVisualSource | undefined {
  const requested = cue ? entity.poses[presentationCuePose(cue)] : undefined;
  return requested ?? entity.poses.idle;
}
