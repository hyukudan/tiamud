import type { SceneImage } from './protocol';

const LANTERN_SQUARE_ASSET =
  '/assets/8ade88f0df1adb294333e72d3fa2d0863543dc0263ebe3038208f3c9e415924c';

/**
 * The entry artwork is deliberately static: it is available before a world
 * session exists and remains independent from authentication and character
 * selection.
 */
export const ENTRY_IMAGE: SceneImage = {
  alt: 'Lantern Square after rain, its dark stones lit by a blue-glass lantern beneath the clock tower.',
  avif: `${LANTERN_SQUARE_ASSET}/scene-1280.avif`,
  avifSrcset: `${LANTERN_SQUARE_ASSET}/scene-640.avif 640w, ${LANTERN_SQUARE_ASSET}/scene-1280.avif 1280w`,
  webp: `${LANTERN_SQUARE_ASSET}/scene-1280.webp`,
  webpSrcset: `${LANTERN_SQUARE_ASSET}/scene-640.webp 640w, ${LANTERN_SQUARE_ASSET}/scene-1280.webp 1280w`,
  jpeg: `${LANTERN_SQUARE_ASSET}/scene-1280.jpg`,
  jpegSrcset: `${LANTERN_SQUARE_ASSET}/scene-640.jpg 640w, ${LANTERN_SQUARE_ASSET}/scene-1280.jpg 1280w`,
  width: 1280,
  height: 560,
};
