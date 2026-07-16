import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';
import App from './App.svelte';
import appSource from './App.svelte?raw';

describe('Tiamud entry flow', () => {
  it('shows account access at the illustrated gate and does not expose guest entry', () => {
    const { body } = render(App);

    expect(body).toContain('<h1 id="welcome-title">Tiamud</h1>');
    expect(body).toContain('Return to your story');
    expect(body).not.toContain('Enter Tiamud');
    expect(body).not.toContain('Awaiting the first scene');
  });

  it('exposes one named live log instead of mirroring world events into the notice region', () => {
    expect(appSource).toContain('role="log"');
    expect(appSource).toContain('aria-label="Command and world event history"');
    expect(appSource).not.toContain("notice = { level: 'info', message: message.data.text }");
  });

  it('keeps a 500-entry anchored scrollback with an explicit latest affordance', () => {
    expect(appSource).toContain('MAX_SCROLLBACK_ENTRIES');
    expect(appSource).toContain('isScrollAnchored');
    expect(appSource).toContain('Jump to latest');
  });
});
