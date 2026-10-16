import { describe, expect, it } from 'vitest';
import fixture from '../../e2e/fixtures/story-cripta.json';
import slotFixture from '../../e2e/fixtures/story-slots.json';
import {
  createStoryChoose,
  createStoryContinue,
  createStoryDeleteSlot,
  createStoryDialogue,
  createStoryLeave,
  createStoryListSlots,
  createStoryMenu,
  createStoryRestore,
  createStorySave,
  createStoryUndo,
  parseServerMessage,
} from './protocol';
import {
  StoryViewAssembler,
  parseStoryEntered,
  parseStoryError,
  parseStoryLeft,
  parseStoryRestored,
  parseStorySaved,
  parseStorySlots,
  parseStoryView,
  presentStory,
} from './story-protocol';

const VERB = 'c1111111111111111';
const TARGET = 'c2222222222222222';
const OPTION = 'c3333333333333333';
const AUDIO = `/assets/${'a'.repeat(64)}/audio-lluvia.ogg`;

const menuView = (overrides: Record<string, unknown> = {}) => ({
  viewRev: 4,
  part: 0,
  parts: 1,
  turn: 3,
  canUndo: true,
  items: [
    { type: 'scene', title: 'Lluvia de neón', text: 'La oficina.\nLlueve.', score: 10, ending: null },
    { type: 'cue', action: 'play', channel: 'music', url: AUDIO, loop: true },
    { type: 'portrait', name: 'Lola' },
    { type: 'block', kind: 'event', text: 'Obtienes: Tarjeta' },
    { type: 'block', kind: 'line', speaker: 'Lola', text: '¿Café?' },
    { type: 'stat', label: 'Pistas', value: '2' },
    { type: 'item', name: 'Tarjeta de memoria' },
    { type: 'verb', id: VERB, label: 'MIRAR', needsTarget: true },
    { type: 'target', verb: VERB, id: TARGET, label: 'Ventana' },
    { type: 'dialogueOption', id: OPTION, label: 'Despedirte' },
  ],
  ...overrides,
});

describe('story protocol', () => {
  it('accepts every real server frame of the sample gamebook fixture', () => {
    const assembler = new StoryViewAssembler();
    for (const step of fixture.steps) {
      for (const part of step.view) {
        const parsed = parseStoryView(part as Record<string, unknown>);
        expect(parsed).toBeDefined();
        if (parsed) assembler.push(parsed);
      }
    }
    const last = fixture.steps.at(-1)!.view[0] as Record<string, unknown>;
    const ending = presentStory(new StoryViewAssembler().push(parseStoryView(last)!)!);
    expect(ending.ending).toBe('neutral');
    expect(ending.choices).toEqual([]);
  });

  it('parses menus, dialogue, speakers and reviewed cues', () => {
    const view = parseStoryView(menuView());
    expect(view).toBeDefined();
    const model = presentStory(new StoryViewAssembler().push(view!)!);
    expect(model.paragraphs).toEqual(['La oficina.', 'Llueve.']);
    expect(model.lines).toEqual([{ speaker: 'Lola', text: '¿Café?' }]);
    expect(model.events).toEqual(['Obtienes: Tarjeta']);
    expect(model.verbs).toEqual([
      { id: VERB, label: 'MIRAR', needsTarget: true, targets: [{ id: TARGET, label: 'Ventana' }] },
    ]);
    expect(model.dialogue).toEqual([{ id: OPTION, label: 'Despedirte' }]);
    expect(model.present).toEqual(['Lola']);
  });

  it('rejects forged shapes, unsafe media and extra fields', () => {
    const scene = menuView().items[0];
    for (const items of [
      [scene, { type: 'choice', id: 't0c0', label: 'x' }],
      [scene, { type: 'choice', id: VERB, label: 'x', node: 'secret' }],
      [scene, { type: 'cue', action: 'play', channel: 'music', url: 'https://evil.test/a.ogg', loop: true }],
      [scene, { type: 'cue', action: 'stop', channel: 'music', url: AUDIO, loop: false }],
      [scene, { type: 'block', kind: 'shout', text: 'x' }],
      [scene, { type: 'verb', id: VERB, label: 'MIRAR' }],
      [scene, { type: 'unknown' }],
      [{ ...scene, ending: { kind: 'won' } }],
    ]) {
      expect(parseStoryView(menuView({ items }))).toBeUndefined();
    }
    expect(parseStoryView(menuView({ part: 1, parts: 1 }))).toBeUndefined();
    expect(parseStoryView(menuView({ viewRev: 0 }))).toBeUndefined();
    expect(parseStoryView(menuView({ extra: true }))).toBeUndefined();
    expect(parseStoryView(menuView({ items: 'x' }))).toBeUndefined();
  });

  it('assembles parts in order and refuses mixed or inconsistent views', () => {
    const [scene, ...rest] = menuView().items;
    const first = parseStoryView(menuView({ parts: 2, items: [scene] }))!;
    const second = parseStoryView(menuView({ part: 1, parts: 2, items: rest }))!;
    const assembler = new StoryViewAssembler();
    expect(assembler.push(first)).toBeUndefined();
    expect(assembler.push(second)?.items).toHaveLength(menuView().items.length);

    // A part of another revision restarts the buffer.
    expect(assembler.push(first)).toBeUndefined();
    expect(assembler.push(parseStoryView(menuView({ viewRev: 9, part: 1, parts: 2, items: rest }))!))
      .toBeUndefined();
    // Targets must name a verb of the same view and ids must be unique.
    expect(new StoryViewAssembler().push(parseStoryView(menuView({
      items: [scene, { type: 'target', verb: VERB, id: TARGET, label: 'x' }],
    }))!)).toBeUndefined();
    expect(new StoryViewAssembler().push(parseStoryView(menuView({
      items: [scene, { type: 'choice', id: OPTION, label: 'a' }, { type: 'dialogueOption', id: OPTION, label: 'b' }],
    }))!)).toBeUndefined();
  });

  it('parses the control messages strictly', () => {
    expect(parseStoryEntered({ storyId: 'lluvia-de-neon', title: 'Lluvia' })).toBeDefined();
    expect(parseStoryEntered({ storyId: 'Bad Id', title: 'Lluvia' })).toBeUndefined();
    expect(parseStoryLeft({ reason: 'evicted' })).toEqual({ reason: 'evicted' });
    expect(parseStoryLeft({ reason: 'gone' })).toBeUndefined();
    expect(parseStoryError({ code: 'stale_view', message: 'x' })).toBeDefined();
    expect(parseStoryError({ code: 'teapot', message: 'x' })).toBeUndefined();
    const raw = JSON.stringify({ v: 1, type: 'story.view', seq: 5, data: menuView() });
    expect(parseServerMessage(raw)?.type).toBe('story.view');
    expect(parseServerMessage(JSON.stringify({ v: 1, type: 'story.view', seq: 5, data: { viewRev: 1 } })))
      .toBeUndefined();
  });

  it('builds strict client messages and refuses non-opaque ids', () => {
    expect(createStoryChoose(OPTION, 2)).toEqual({ v: 1, type: 'story.choose', data: { id: OPTION, viewRev: 2 } });
    expect(createStoryDialogue(OPTION, 2).type).toBe('story.dialogue');
    expect(createStoryMenu(VERB, undefined, 2).data).toEqual({ verb: VERB, viewRev: 2 });
    expect(createStoryMenu(VERB, TARGET, 2).data).toEqual({ verb: VERB, target: TARGET, viewRev: 2 });
    expect(createStoryUndo().data).toEqual({});
    expect(createStoryLeave().type).toBe('story.leave');
    expect(() => createStoryChoose('t0c0', 2)).toThrow();
    expect(() => createStoryChoose(OPTION, 0)).toThrow();
    expect(() => createStoryMenu(VERB, 'nope', 1)).toThrow();
  });

  it('accepts the recorded save-slot frames and builds slot messages', () => {
    for (const data of [slotFixture.enter.slots, slotFixture.slots.slots, slotFixture.reenter.slots]) {
      expect(parseStorySlots(data)).toEqual(data);
    }
    expect(parseStorySaved(slotFixture.save.saved)).toEqual(slotFixture.save.saved);
    expect(parseStoryRestored(slotFixture.continue.restored)).toEqual({ slot: 0 });
    for (const part of slotFixture.continue.view) expect(parseStoryView(part)).toBeDefined();
    const raw = JSON.stringify({ v: 1, type: 'story.slots', seq: 9, data: slotFixture.reenter.slots });
    expect(parseServerMessage(raw)?.type).toBe('story.slots');
    expect(parseServerMessage(JSON.stringify({ v: 1, type: 'story.saved', seq: 9, data: { slot: 0, updatedAt: 1 } })))
      .toBeUndefined();

    expect(createStorySave(3)).toEqual({ v: 1, type: 'story.save', data: { slot: 3 } });
    expect(createStoryRestore(0).data).toEqual({ slot: 0 });
    expect(createStoryDeleteSlot(9).type).toBe('story.deleteSlot');
    expect(createStoryListSlots()).toEqual({ v: 1, type: 'story.slots', data: {} });
    expect(createStoryContinue()).toEqual({ v: 1, type: 'story.continue', data: {} });
    expect(() => createStorySave(0)).toThrow();
    expect(() => createStorySave(10)).toThrow();
    expect(() => createStoryRestore(1.5)).toThrow();
    expect(() => createStoryDeleteSlot(-1)).toThrow();
  });

  it('rejects forged, duplicated or inconsistent slot listings', () => {
    const slot = (value: number, compatible = true) => ({ slot: value, updatedAt: 10, compatible });
    expect(parseStorySlots({ slots: [slot(0), slot(2)], offerContinue: true })).toBeDefined();
    for (const data of [
      { slots: [slot(2), slot(0)], offerContinue: false },
      { slots: [slot(2), slot(2)], offerContinue: false },
      { slots: [slot(10)], offerContinue: false },
      { slots: [slot(1)], offerContinue: true },
      { slots: [slot(0, false)], offerContinue: true },
      { slots: [{ ...slot(1), stateJson: '{}' }], offerContinue: false },
      { slots: [{ ...slot(1), updatedAt: -1 }], offerContinue: false },
      { slots: [], offerContinue: false, extra: 1 },
      { slots: Array.from({ length: 11 }, (_, index) => slot(index)), offerContinue: false },
    ]) expect(parseStorySlots(data)).toBeUndefined();
    expect(parseStorySaved({ slot: 1, updatedAt: 2, extra: true })).toBeUndefined();
    expect(parseStoryRestored({ slot: 11 })).toBeUndefined();
    for (const code of ['slot_empty', 'slot_corrupt', 'slot_incompatible', 'rate_limited']) {
      expect(parseStoryError({ code, message: 'x' })?.code).toBe(code);
    }
  });
});
