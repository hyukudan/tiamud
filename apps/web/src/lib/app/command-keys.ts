export type CommandKeyEvent = Pick<
  KeyboardEvent,
  'key' | 'keyCode' | 'isComposing' | 'shiftKey' | 'altKey' | 'ctrlKey' | 'metaKey' | 'defaultPrevented'
>;

/**
 * True for an unmodified Enter that should send the command line.
 *
 * The command field submits on keydown instead of relying on implicit form
 * submission: WebKit's `<datalist>` suggestion handling can consume Enter in
 * its keydown default action, which suppresses `keypress` and therefore the
 * implicit submit, leaving the command unsent. An Enter that confirms an IME
 * composition (`isComposing`, or the legacy 229 key code) is never treated as
 * a submission, and modified Enter keeps its browser meaning.
 */
export function isSubmitEnter(event: CommandKeyEvent): boolean {
  return event.key === 'Enter' &&
    !event.defaultPrevented &&
    !event.isComposing &&
    event.keyCode !== 229 &&
    !event.shiftKey &&
    !event.altKey &&
    !event.ctrlKey &&
    !event.metaKey;
}
