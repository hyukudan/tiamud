import { describe, expect, it } from 'vitest';
import fixture from '../../e2e/fixtures/story-lighthouse.json';
import { createStoryCommand } from './protocol';
import {
  MAX_STORY_COMMAND_BYTES,
  StoryViewAssembler,
  parseStoryError,
  parseStoryItem,
  parseStoryView,
  presentStory,
} from './story-protocol';

describe('parser story protocol (StoryPack v3)', () => {
  it('accepts every real server frame of the recorded parser adventure', () => {
    const assembler = new StoryViewAssembler();
    let questions = 0;
    for (const step of fixture.steps) {
      let assembled;
      for (const part of step.view) {
        const parsed = parseStoryView(part as Record<string, unknown>);
        expect(parsed).toBeDefined();
        assembled = assembler.push(parsed!);
      }
      expect(assembled).toBeDefined();
      const model = presentStory(assembled!);
      if (step.input) {
        expect(model.prompt?.text).toBe('>');
        if (model.prompt?.question) questions += 1;
        // The client builds exactly the recorded wire message.
        const built = createStoryCommand(step.input.data.text, step.input.data.viewRev);
        expect({ type: built.type, data: built.data }).toEqual(step.input);
      } else {
        expect(model.prompt).toBeUndefined();
        expect(model.ending).toBe('victory');
        expect(model.command).toBe('the great lamp');
      }
    }
    expect(questions).toBe(1);
  });

  it('presents the echo, replies and prompt separately from narration', () => {
    const model = presentStory({
      viewRev: 4,
      turn: 2,
      canUndo: true,
      items: [
        {
          type: 'scene', title: 'Faro', text: 'Linterna', score: 0, ending: null,
          prompt: { text: '>', question: true },
        },
        { type: 'block', kind: 'command', text: 'enciende la lampara' },
        { type: 'block', kind: 'reply', text: '¿Cuál quieres decir?' },
      ],
    });
    expect(model.prompt).toEqual({ text: '>', question: true });
    expect(model.command).toBe('enciende la lampara');
    expect(model.replies).toEqual(['¿Cuál quieres decir?']);
    expect(model.lines).toEqual([]);
  });

  it('rejects forged prompts, block kinds and unknown error codes', () => {
    const scene = { type: 'scene', title: 'T', text: 'x', score: 0, ending: null };
    expect(parseStoryItem({ ...scene, prompt: { text: '>', question: false } })).toBeDefined();
    for (const prompt of [
      { text: '>' },
      { text: '', question: false },
      { text: '>', question: 'yes' },
      { text: '>', question: false, extra: 1 },
      { text: 'x'.repeat(65), question: false },
      null,
    ]) expect(parseStoryItem({ ...scene, prompt })).toBeUndefined();
    expect(parseStoryItem({ type: 'block', kind: 'shout', text: 'x' })).toBeUndefined();
    expect(parseStoryItem({ type: 'block', kind: 'reply', text: '' })).toBeUndefined();
    expect(parseStoryError({ code: 'no_parser', message: 'x' })?.code).toBe('no_parser');
    expect(parseStoryError({ code: 'invalid_command', message: 'x' })?.code).toBe('invalid_command');
  });

  it('builds bounded story.command messages', () => {
    expect(createStoryCommand('  coge la llave ', 3)).toEqual({
      v: 1, type: 'story.command', data: { text: 'coge la llave', viewRev: 3 },
    });
    expect(createStoryCommand('ñ'.repeat(128), 1).data.text).toHaveLength(128);
    for (const [text, viewRev] of [
      ['', 1], ['   ', 1], ['n', 0], ['n', 1.5], ['ñ'.repeat(129), 1],
      ['a'.repeat(MAX_STORY_COMMAND_BYTES + 1), 1],
    ] as const) {
      expect(() => createStoryCommand(text, viewRev)).toThrow();
    }
  });
});
