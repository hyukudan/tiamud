import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';
import WelcomeGate from './WelcomeGate.svelte';
import welcomeSource from './WelcomeGate.svelte?raw';

describe('WelcomeGate', () => {
  it('introduces Tiamud with a clear entry action and complete text', () => {
    const { body } = render(WelcomeGate, { props: { onenter: () => undefined } });
    const text = body.replace(/\s+/g, ' ');

    expect(body).toContain('<h1 id="welcome-title">Tiamud</h1>');
    expect(body).toContain('Enter Tiamud');
    expect(text).toContain('fully described in text');
    expect(body).toContain('data-auth-boundary');
  });

  it('serves the entry illustration as AVIF, WebP, and JPEG with meaningful alt text', () => {
    const { body } = render(WelcomeGate, { props: { onenter: () => undefined } });

    expect(body).toContain('type="image/avif"');
    expect(body).toContain('type="image/webp"');
    expect(body).toContain('scene-1280.jpg');
    expect(body).toContain('Lantern Square after rain');
    expect(welcomeSource).toContain('imageAfterFailures(ENTRY_IMAGE, failedArtworkUrls)');
    expect(welcomeSource).toContain('failedArtworkUrls = [...failedArtworkUrls, failed]');
    expect(welcomeSource).toContain('{#key artworkRenderKey}');
  });
});
