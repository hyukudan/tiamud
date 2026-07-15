# Tiamud for TinTin++

Load the profile after connecting with:

```text
#read clients/tintin/tiamud.tin
```

The script stores `Room.Info`, `Tiamud.Scene`, and `Tiamud.Assets` in the
`TIAMUD` variable tree. It does not execute server-provided text or movement
commands. Media remains off until `tiamud-media-on` is entered explicitly;
even then, only `/assets/<64 lowercase hex>/<file>.opus|ogg|mp3` events are
retained. The package never downloads or plays them.

CellFrame output is off by default. Explicit commands select one renderer:
`tiamud-graphics-ansi`, `tiamud-graphics-kitty`,
`tiamud-graphics-sixel`, or `tiamud-graphics-iterm2`.
`tiamud-graphics-off` revokes consent. TinTin++ and the terminal must support
the selected renderer.

`tiamud-accessible` enables the conservative plain, reduced-motion,
screen-reader and decorative-off policy; it also revokes graphics and media.
