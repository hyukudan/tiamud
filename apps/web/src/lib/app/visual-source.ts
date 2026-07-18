import type { SceneEntityVisualSource } from '../protocol';

export function directVisualSource(
  source: SceneEntityVisualSource | undefined,
): SceneEntityVisualSource | undefined {
  return source && (source.avif || source.webp || source.png) ? source : undefined;
}

export function visualSourceUrl(source: SceneEntityVisualSource): string {
  return source.png ?? source.webp ?? source.avif ?? '';
}
