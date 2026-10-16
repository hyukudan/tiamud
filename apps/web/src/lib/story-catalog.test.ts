import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';
import catalogFixture from '../../e2e/fixtures/story-catalog.json';
import lluvia from '../../e2e/fixtures/story-lluvia.json';
import StoryPanel from '../components/StoryPanel.svelte';
import StoryPicker from '../components/StoryPicker.svelte';
import { createStoryCatalog, parseServerMessage } from './protocol';
import {
  MAX_CATALOG_STORIES,
  StoryCatalogAssembler,
  StoryViewAssembler,
  parseStoryCatalog,
  parseStoryEntered,
  parseStoryItem,
  parseStoryView,
  preferredStoryLocale,
  presentStory,
  type AssembledStoryView,
} from './story-protocol';

const PORTRAIT = { manifest: `/assets/${'b'.repeat(64)}/manifest.json`, alt: 'Silueta de Lola.' };
const entry = (storyId: string, locales: string[] = ['es']) => ({ storyId, title: storyId, locales });

describe('story catalog with languages', () => {
  it('parses the recorded server catalog and its English entry', () => {
    const [part] = catalogFixture.catalog;
    const parsed = parseStoryCatalog(part as Record<string, unknown>);
    expect(parsed?.stories.map((story) => [story.storyId, story.locales])).toEqual([
      ['cripta-del-nigromante', ['es']],
      ['lluvia-de-neon', ['es', 'en']],
    ]);
    expect(parseStoryEntered(catalogFixture.entered as Record<string, unknown>))
      .toMatchObject({ storyId: 'lluvia-de-neon', title: 'Neon Rain', locale: 'en' });
    const envelope = parseServerMessage(JSON.stringify({ v: 1, type: 'story.catalog', seq: 3, data: part }));
    expect(envelope?.type).toBe('story.catalog');
    expect(createStoryCatalog()).toEqual({ v: 1, type: 'story.catalog', data: {} });
  });

  it('rejects malformed catalogs, entries and locale lists', () => {
    const valid = { part: 0, parts: 1, stories: [entry('cripta')] };
    expect(parseStoryCatalog(valid)).toEqual(valid);
    for (const bad of [
      { ...valid, extra: 1 },
      { ...valid, part: 1 },
      { ...valid, parts: 0 },
      { ...valid, parts: 65 },
      { ...valid, stories: [{ ...entry('cripta'), url: '/x' }] },
      { ...valid, stories: [entry('Cripta')] },
      { ...valid, stories: [entry('cripta', [])] },
      { ...valid, stories: [entry('cripta', ['es', 'es'])] },
      { ...valid, stories: [entry('cripta', ['EN'])] },
      { ...valid, stories: [entry('cripta', Array.from({ length: 17 }, (_, i) => `l${i}x`))] },
      { ...valid, stories: [{ ...entry('cripta'), title: '' }] },
      { ...valid, stories: Array.from({ length: MAX_CATALOG_STORIES + 1 }, (_, i) => entry(`s${i}`)) },
    ]) expect(parseStoryCatalog(bad as Record<string, unknown>)).toBeUndefined();
  });

  it('assembles parts in order and refuses duplicates', () => {
    const assembler = new StoryCatalogAssembler();
    expect(assembler.push({ part: 0, parts: 2, stories: [entry('a')] })).toBeUndefined();
    expect(assembler.push({ part: 1, parts: 2, stories: [entry('b', ['es', 'en'])] }))
      .toEqual([entry('a'), entry('b', ['es', 'en'])]);
    // A part out of order restarts the buffer instead of mixing listings.
    expect(assembler.push({ part: 1, parts: 2, stories: [entry('b')] })).toBeUndefined();
    expect(assembler.push({ part: 0, parts: 2, stories: [entry('a')] })).toBeUndefined();
    expect(assembler.push({ part: 1, parts: 2, stories: [entry('a')] })).toBeUndefined();
    expect(assembler.push({ part: 0, parts: 1, stories: [] })).toEqual([]);
  });

  it('preselects the reader preference when offered, else Spanish, else the default', () => {
    expect(preferredStoryLocale(['es', 'en'], ['en-US', 'es'])).toBe('en');
    expect(preferredStoryLocale(['es', 'en'], ['EN'])).toBe('en');
    expect(preferredStoryLocale(['es', 'en'], ['fr-FR', 'de'])).toBe('es');
    expect(preferredStoryLocale(['en', 'es'], [])).toBe('es');
    expect(preferredStoryLocale(['en', 'ca'], ['fr'])).toBe('en');
    expect(preferredStoryLocale(['es'], ['en'])).toBe('es');
    expect(preferredStoryLocale(['es', 'en'], [42, 'x'.repeat(64), 'en'])).toBe('en');
  });

  it('renders an accessible picker with one language select per bilingual story', () => {
    const { body } = render(StoryPicker, {
      props: {
        stories: [entry('cripta'), { storyId: 'lluvia', title: 'Lluvia de neón', locales: ['es', 'en'] }],
        connected: true,
        preferred: ['en-GB'],
        onenter: () => undefined,
        onclose: () => undefined,
      },
    });
    expect(body).toContain('aria-labelledby="story-picker-title"');
    expect(body).toContain('<select id="story-picker-lluvia-locale"');
    expect(body).toContain('for="story-picker-lluvia-locale"');
    // The English preference is offered and preselected; options name
    // each language in itself.
    expect(body).toMatch(/<option value="en" lang="en"[^>]*selected/);
    expect(body).not.toContain('<select id="story-picker-cripta-locale"');
    expect(body).toContain('Play');
    expect(body).toContain('Close stories');
  });
});

describe('dialogue line portraits', () => {
  it('accepts a portrait only on a spoken line', () => {
    const line = { type: 'block', kind: 'line', speaker: 'Lola', text: '¿Café?' };
    expect(parseStoryItem({ ...line, portrait: PORTRAIT })).toEqual({ ...line, portrait: PORTRAIT });
    for (const bad of [
      { ...line, portrait: { manifest: '/x.png', alt: 'x' } },
      { ...line, portrait: null },
      { type: 'block', kind: 'line', text: 'Sin hablante.', portrait: PORTRAIT },
      { type: 'block', kind: 'event', speaker: 'Lola', text: 'x', portrait: PORTRAIT },
      { type: 'block', kind: 'reply', text: 'x', portrait: PORTRAIT },
    ]) expect(parseStoryItem(bad)).toBeUndefined();
  });

  it('presents recorded line portraits and renders them only with images on', () => {
    const assembler = new StoryViewAssembler();
    let withPortrait: AssembledStoryView | undefined;
    for (const step of lluvia.steps) {
      let assembled: AssembledStoryView | undefined;
      for (const part of step.view) assembled = assembler.push(parseStoryView(part as Record<string, unknown>)!) ?? assembled;
      if (presentStory(assembled!).lines.some((line) => line.portrait)) {
        withPortrait = assembled;
        break;
      }
    }
    expect(withPortrait).toBeDefined();
    const story = presentStory(withPortrait!);
    const portrayed = story.lines.find((line) => line.portrait)!;
    expect(portrayed.speaker).toBe('Lola');
    expect(portrayed.portrait?.alt).toContain('Lola');

    const props = {
      view: withPortrait!, connected: true, pending: false,
      onchoose: () => undefined, onmenu: () => undefined, onundo: () => undefined, onleave: () => undefined,
    };
    // Server rendering cannot fetch the manifest, but the row is laid out.
    expect(render(StoryPanel, { props: { ...props, images: true } }).body).toContain('story-line-row');
    // Reader mode, Save-Data or multimedia off: no row, the speaker name stays.
    const plain = render(StoryPanel, { props: { ...props, images: false } }).body;
    expect(plain).not.toContain('story-line-row');
    expect(plain).toContain('<strong>Lola:</strong>');
  });
});
