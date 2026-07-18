import type { PlayerStateSnapshot } from '../protocol';

export const statusLabel = (status: PlayerStateSnapshot['quests'][number]['status']): string => ({
  available: 'Available',
  active: 'Active',
  readyToClaim: 'Reward ready',
  completed: 'Completed',
})[status];

export const humanizeId = (id: string): string => id.split(/[_:.-]+/).filter(Boolean)
  .map((part) => (part[0]?.toUpperCase() ?? '') + part.slice(1))
  .join(' ');

export function exactProbability(numerator: number, denominator: number): string {
  return `${numerator}/${denominator} (${((numerator * 100) / denominator).toFixed(2)}%)`;
}

export function signedDelta(value: number): string {
  return `${value >= 0 ? '+' : ''}${value}`;
}

export function resourceCostLabel(
  cost: PlayerStateSnapshot['character']['abilities'][number]['resourceCosts'][number],
): string {
  return `${cost.amount} ${cost.label}`;
}
