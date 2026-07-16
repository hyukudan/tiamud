# Medios atmosféricos y sprites de entidad

Los medios acompañan a la prosa; nunca contienen información jugable
exclusiva. El servidor resuelve primero la percepción y solo envía medios de la
escena y de entidades que el personaje ya puede conocer. Telnet y las sesiones
de navegador sin imágenes siguen siendo completas.

## Escenas atmosféricas

La ilustración opaca de escena se compila fuera de línea a variantes AVIF
preferentes para el navegador, con alternativas WebP y JPEG. Las fuentes
atmosféricas transparentes o explícitamente lossless usan PNG. Las salas
referencian un manifiesto revisado y direccionado por contenido; `roomMedia`
aporta texto alternativo localizado y audio opcional. `sceneVariants` puede
seleccionar otro manifiesto autorizado según condiciones acotadas de tiempo,
clima, misión y percepción.

La política exacta de codificación y los comandos están en la
[guía del pipeline de recursos](../../crates/asset-pipeline/README.md).

## Sprites de entidad

`entityMedia` decora opcionalmente un prototipo reusable de objeto o un spawn de
NPC existentes; no crea una entidad. Para `kind: item`, `subjectId` es el ID del
**prototipo**, no el de una instancia: el servidor reutiliza la definición para
cada instancia de ese prototipo que ya sea visible. Para `kind: npc`,
`subjectId` es el ID del spawn. En cambio, `roomPerception.hiddenItems` usa IDs
de **instancia**, por lo que una unidad puede permanecer oculta sin ocultar las
demás del mismo prototipo.

La pose `idle` es obligatoria; `attack`, `hit` y `defeated` son opcionales. Cada
pose solo puede apuntar a ficheros WebP con alfa y/o PNG estrictos, del mismo
proyecto, direccionados por contenido y generados con el perfil
`entity-sprite`.

```json
{
  "kind": "npc",
  "subjectId": "demo:gate-stalker-1",
  "decorative": true,
  "footprint": { "widthMilli": 220, "heightMilli": 420 },
  "preferredAnchor": { "xMilli": 700, "yMilli": 850 },
  "priority": 20,
  "layer": "foreground",
  "poses": {
    "idle": {
      "webp": "assets/<sha256>/sprite-idle-640.webp",
      "png": "assets/<sha256>/sprite-idle-640.png"
    },
    "hit": {
      "webp": "assets/<sha256>/sprite-hit-640.webp",
      "png": "assets/<sha256>/sprite-hit-640.png"
    }
  }
}
```

Sustituye `<sha256>` por el ID de bundle de 64 caracteres hexadecimales en
minúsculas emitido por `tiamud-assets`. Un sprite decorativo debe omitir `alt`;
uno no decorativo requiere texto alternativo localizado válido. Las coordenadas
y la huella usan milésimas de la escena, no píxeles CSS.

```sh
cargo run --locked -p tiamud-asset-pipeline --bin tiamud-assets -- \
  build --kind entity-sprite --pose idle \
  --source artwork/gate-stalker-idle.png \
  --alt "A hunched gate-stalker watching the road." \
  --output apps/web/public/assets \
  --author "Tiamud contributors" \
  --license "CC-BY-4.0" \
  --provenance "Reviewed original transparent illustration." \
  --reviewed-at "2026-07-16T00:00:00Z"
```

Compila cada pose por separado y copia las rutas emitidas exactas en el
AreaPack propietario. Los sprites de entidad usan actualmente WebP alfa
estático con alternativa PNG; AVIF con alfa sigue aplazado. No se aceptan SVG,
URL remotas, datos inline ni scripts de animación.

## Comportamiento del navegador

El navegador usa colocación responsive determinista y puede omitir decoración
según los presupuestos de densidad, ahorro de datos, accesibilidad o
decodificación. Las listas semánticas de entidades y los controles nativos de
acción permanecen. Las pistas cosméticas de combate solo pueden elegir una pose
ya creada y nunca afectan al tiempo ni al estado del combate.

Consulta el [diseño del compositor de escena](../design/scene-compositor-v1.md)
para las reglas de layout, overflow, movimiento y accesibilidad.
