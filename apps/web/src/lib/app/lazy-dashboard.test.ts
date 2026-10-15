import { afterEach, describe, expect, it, vi } from 'vitest';
import { loadPlayerDashboard, resetPlayerDashboardLoader } from './lazy-dashboard';

afterEach(() => resetPlayerDashboardLoader());

describe('lazy player dashboard', () => {
  it('shares one in-flight import', async () => {
    const component = (() => undefined) as never;
    const importer = vi.fn(async () => ({ default: component }));
    const [first, second] = await Promise.all([
      loadPlayerDashboard(importer),
      loadPlayerDashboard(importer),
    ]);
    expect(first).toBe(component);
    expect(second).toBe(component);
    expect(importer).toHaveBeenCalledOnce();
  });

  it('forgets a failed import so an explicit retry can succeed', async () => {
    const component = (() => undefined) as never;
    const importer = vi.fn()
      .mockRejectedValueOnce(new Error('offline'))
      .mockResolvedValueOnce({ default: component });
    await expect(loadPlayerDashboard(importer)).rejects.toThrow('offline');
    await expect(loadPlayerDashboard(importer)).resolves.toBe(component);
    expect(importer).toHaveBeenCalledTimes(2);
  });

  it('resolves the real component module', async () => {
    const component = await loadPlayerDashboard();
    expect(typeof component).toBe('function');
  });
});
