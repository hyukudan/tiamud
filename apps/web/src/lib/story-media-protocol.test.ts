import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';
import lluvia from '../../e2e/fixtures/story-lluvia.json';
import StoryPanel from '../components/StoryPanel.svelte';
import {
  StoryViewAssembler,
  parseStoryEntered,
  parseStoryError,
  parseStoryImage,
  parseStoryItem,
  parseStoryView,
  presentStory,
  type AssembledStoryView,
} from './story-protocol';

const MANIFEST = `/assets/${'a'.repeat(64)}/manifest.json`;
const PORTRAIT = `/assets/${'b'.repeat(64)}/manifest.json`;

const scene = (extra: Record<string, unknown> = {}) => ({
  type: 'scene', title: 'Lluvia de neón', text: 'La oficina.', score: 0, ending: null, ...extra,
});

describe('story images and locales', () => {
  it('accepts reviewed manifests with alt text and rejects anything else', () => {
    expect(parseStoryImage({ manifest: MANIFEST, alt: 'Una oficina.' }))
      .toEqual({ manifest: MANIFEST, alt: 'Una oficina.' });
    for (const bad of [
      { manifest: 'https://example.test/a.jpg', alt: 'x' },
      { manifest: `/assets/${'a'.repeat(64)}/scene-640.jpg`, alt: 'x' },
      { manifest: `assets/${'a'.repeat(64)}/manifest.json`, alt: 'x' },
      { manifest: MANIFEST, alt: '' },
      { manifest: MANIFEST, alt: '   ' },
      { manifest: MANIFEST, alt: 'uno\ndos' },
      { manifest: MANIFEST, alt: 'x'.repeat(513) },
      { manifest: MANIFEST },
      { manifest: MANIFEST, alt: 'x', url: '/x.jpg' },
    ]) expect(parseStoryImage(bad)).toBeUndefined();

    expect(parseStoryItem(scene({ background: { manifest: MANIFEST, alt: 'Una oficina.' } })))
      .toMatchObject({ background: { manifest: MANIFEST, alt: 'Una oficina.' } });
    // A malformed image fails the whole item, like every other field.
    expect(parseStoryItem(scene({ background: { manifest: '/x.png', alt: 'x' } }))).toBeUndefined();
    expect(parseStoryItem({ type: 'portrait', name: 'Lola', image: { manifest: PORTRAIT, alt: 'Lola.' } }))
      .toEqual({ type: 'portrait', name: 'Lola', image: { manifest: PORTRAIT, alt: 'Lola.' } });
    expect(parseStoryItem({ type: 'portrait', name: 'Lola', image: null })).toBeUndefined();
    expect(parseStoryItem({ type: 'portrait', name: 'Lola' })).toEqual({ type: 'portrait', name: 'Lola' });
  });

  it('presents backgrounds and portraits from real recorded frames', () => {
    const assembler = new StoryViewAssembler();
    const backgrounds = new Set<string>();
    const portraits = new Set<string>();
    for (const step of lluvia.steps) {
      let assembled: AssembledStoryView | undefined;
      for (const part of step.view) {
        const parsed = parseStoryView(part as Record<string, unknown>);
        expect(parsed).toBeDefined();
        assembled = assembler.push(parsed!) ?? assembled;
      }
      const story = presentStory(assembled!);
      if (story.background) backgrounds.add(story.background.manifest);
      for (const portrait of story.portraits) {
        if (portrait.image) portraits.add(`${portrait.name}:${portrait.image.manifest}`);
      }
    }
    // The office and club backgrounds plus Lola's and Dámaso's portraits.
    expect(backgrounds.size).toBe(2);
    expect([...portraits].map((entry) => entry.split(':')[0]).sort()).toEqual(['Dámaso', 'Lola']);
  });

  it('reads the instance locale from story.entered and defaults older servers to Spanish', () => {
    expect(parseStoryEntered({ storyId: 'lluvia-de-neon', title: 'Neon Rain', locale: 'en', locales: ['es', 'en'] }))
      .toEqual({ storyId: 'lluvia-de-neon', title: 'Neon Rain', locale: 'en', locales: ['es', 'en'] });
    expect(parseStoryEntered({ storyId: 'cripta', title: 'Cripta' }))
      .toEqual({ storyId: 'cripta', title: 'Cripta', locale: 'es', locales: ['es'] });
    for (const bad of [
      { storyId: 'x', title: 'X', locale: 'en', locales: ['es'] },
      { storyId: 'x', title: 'X', locale: 'e', locales: ['e'] },
      { storyId: 'x', title: 'X', locale: 'es', locales: ['es', 'es'] },
      { storyId: 'x', title: 'X', locale: 'es', locales: [] },
      { storyId: 'x', title: 'X', locale: 7 },
    ]) expect(parseStoryEntered(bad)).toBeUndefined();
    expect(parseStoryError({ code: 'unknown_locale', message: 'No.' }))
      .toEqual({ code: 'unknown_locale', message: 'No.' });
  });

  it('renders alt text instead of downloading when images are off', () => {
    const view: AssembledStoryView = {
      viewRev: 1, turn: 0, canUndo: false,
      items: [
        { ...scene(), background: { manifest: MANIFEST, alt: 'Una oficina de noche.' } } as never,
        { type: 'portrait', name: 'Lola', image: { manifest: PORTRAIT, alt: 'Silueta de Lola.' } },
      ],
    };
    const base = {
      view, connected: true, pending: false, lang: 'en',
      onchoose: () => undefined, onmenu: () => undefined, onundo: () => undefined, onleave: () => undefined,
    };
    const { body } = render(StoryPanel, { props: base });
    expect(body).toContain('Una oficina de noche.');
    expect(body).not.toContain('<img');
    expect(body).not.toContain('Portraits');
    expect(body).toContain('Present: Lola');
    expect(body).toContain('lang="en"');

    // With images on, nothing is shown before the manifest loads; the text
    // and the names stay.
    const { body: rich } = render(StoryPanel, { props: { ...base, images: true } });
    expect(rich).toContain('aria-label="Portraits"');
    expect(rich).toContain('Present: Lola');
    expect(rich).not.toContain('<img');
  });
});
