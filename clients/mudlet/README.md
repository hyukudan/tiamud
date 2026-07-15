# Tiamud for Mudlet

Create a Mudlet package from this directory, or load `tiamud.lua` as a profile
script. It consumes `Room.Info`, `Tiamud.Scene`, assets, and media events while
keeping all server-provided values out of Lua evaluation and `send()` calls.

Media is disabled by default. The package accepts only root-relative,
content-addressed `/assets/<64 lowercase hex>/<file>.opus|ogg|mp3` events from
the connected Tiamud server. It validates and raises them as local events; it
never downloads, plays, opens, evaluates, or sends anything automatically.
Enable the audited event stream explicitly with:

```lua
Tiamud.enableMedia(true)
```

Links are also opt-in with `Tiamud.enableLinks("mslp")` or
`Tiamud.enableLinks("mxp")`.

Decorative CellFrame output is also off by default. Select exactly one renderer
only if the current Mudlet terminal supports it:

```lua
Tiamud.enableGraphics("ansi")   -- or kitty, sixel, iterm2
Tiamud.enableGraphics("none")   -- revoke consent
```

`Tiamud.setAccessibility(plain, reducedMotion, screenReader, decorative)`
updates the server policy. Plain, reduced-motion, screen-reader, or
`decorative=false` settings force graphics, image assets, media, and rich links
off regardless of previous opt-ins.
