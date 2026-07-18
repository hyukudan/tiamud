import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';
import PreferencesPanel from './PreferencesPanel.svelte';
import { DEFAULT_PREFERENCES, type ClientPreferences } from '../lib/preferences';

describe('PreferencesPanel', () => {
  it('renders the preferences controls', () => {
    const { body } = render(PreferencesPanel, {
      props: { value: DEFAULT_PREFERENCES, onchange: () => undefined },
    });

    expect(body).toContain('Preferences');
    expect(body).toContain('Images');
    expect(body).toContain('Screen-reader mode');
    expect(body).toContain('Text size');
  });

  it('hides the audio volume sliders while audio is disabled', () => {
    const value: ClientPreferences = { ...DEFAULT_PREFERENCES, audio: false };
    const { body } = render(PreferencesPanel, {
      props: { value, onchange: () => undefined },
    });

    expect(body).not.toContain('type="range"');
    expect(body).not.toContain('Audio volumes');
  });

  it('shows the audio volume sliders once audio is enabled', () => {
    const value: ClientPreferences = { ...DEFAULT_PREFERENCES, audio: true };
    const { body } = render(PreferencesPanel, {
      props: { value, onchange: () => undefined },
    });

    expect(body).toContain('type="range"');
    expect(body).toContain('Audio volumes');
  });
});
