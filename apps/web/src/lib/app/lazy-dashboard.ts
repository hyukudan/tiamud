import type PlayerDashboard from '../../components/PlayerDashboard.svelte';

/**
 * The player dashboard is the largest component and is only rendered after
 * entering the world, so it ships as its own chunk. One in-flight import is
 * shared; a failed import is forgotten so an explicit retry can try again.
 */
export type PlayerDashboardComponent = typeof PlayerDashboard;
type Importer = () => Promise<{ default: PlayerDashboardComponent }>;

const importDashboard: Importer = () => import('../../components/PlayerDashboard.svelte');

let pending: Promise<PlayerDashboardComponent> | undefined;

export function loadPlayerDashboard(importer: Importer = importDashboard): Promise<PlayerDashboardComponent> {
  pending ??= importer().then(
    (module) => module.default,
    (error: unknown) => {
      pending = undefined;
      throw error;
    },
  );
  return pending;
}

/** Test seam: forget the cached import. */
export function resetPlayerDashboardLoader(): void {
  pending = undefined;
}
