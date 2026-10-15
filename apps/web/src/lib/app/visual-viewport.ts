/**
 * Keeps the fixed mobile command bar above an on-screen keyboard.
 *
 * Mobile browsers that overlay the keyboard shrink only the *visual*
 * viewport; a `position: fixed; bottom: 0` bar stays anchored to the layout
 * viewport and ends up hidden behind the keys. The covered height is
 * `innerHeight - (visualViewport.height + visualViewport.offsetTop)`, which we
 * publish as `--keyboard-inset` for CSS to lift the bar by. Browsers that
 * resize the layout viewport instead report zero, so the bar does not move
 * twice. Without `visualViewport` this is a no-op.
 */
export const KEYBOARD_INSET_PROPERTY = '--keyboard-inset';

export interface VisualViewportLike {
  readonly height: number;
  readonly offsetTop: number;
  addEventListener(type: 'resize' | 'scroll', listener: () => void): void;
  removeEventListener(type: 'resize' | 'scroll', listener: () => void): void;
}

export interface ViewportWindowLike {
  readonly innerHeight: number;
  readonly visualViewport?: VisualViewportLike | null;
}

export interface StyledElementLike {
  style: Pick<CSSStyleDeclaration, 'setProperty' | 'removeProperty'>;
}

export function keyboardInset(innerHeight: number, viewport: Pick<VisualViewportLike, 'height' | 'offsetTop'>): number {
  const covered = innerHeight - (viewport.height + viewport.offsetTop);
  if (!Number.isFinite(covered) || covered <= 0 || !Number.isFinite(innerHeight)) return 0;
  // Sub-pixel noise from zoom is not a keyboard; never lift past the screen.
  return covered < 1 ? 0 : Math.min(Math.round(covered), Math.max(0, Math.round(innerHeight)));
}

export function keepCommandVisible(view: ViewportWindowLike, element: StyledElementLike): () => void {
  const viewport = view.visualViewport;
  if (!viewport) return () => undefined;
  let applied = -1;
  const update = (): void => {
    const inset = keyboardInset(view.innerHeight, viewport);
    if (inset === applied) return;
    applied = inset;
    if (inset === 0) element.style.removeProperty(KEYBOARD_INSET_PROPERTY);
    else element.style.setProperty(KEYBOARD_INSET_PROPERTY, `${inset}px`);
  };
  viewport.addEventListener('resize', update);
  viewport.addEventListener('scroll', update);
  update();
  return () => {
    viewport.removeEventListener('resize', update);
    viewport.removeEventListener('scroll', update);
    element.style.removeProperty(KEYBOARD_INSET_PROPERTY);
  };
}
