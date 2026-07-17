# Fuentes visuales de `tiamud:gate-stalker`

Estado: **cuarentena pendiente de revisión humana**. Ninguna de estas imágenes
está publicada, enlazada desde un WorldPack ni marcada con `humanReviewedAt`.

## Procedencia

- Fecha de generación: 2026-07-17.
- Generador: herramienta integrada de generación de imágenes de OpenAI; el
  identificador concreto del modelo no fue expuesto por la herramienta.
- Referencia del primer pase: `apps/web/artwork/lantern-square-source.png`,
  usada únicamente como referencia de paleta y lenguaje pictórico. No se pidió
  copiar su escena, arquitectura, composición o linterna.
- Variantes: `attack`, `hit` y `defeated` usaron la imagen `idle` generada como
  ancla de identidad.
- Prompts completos: [`prompts/`](prompts/).
- Fondo de generación: clave cromática plana magenta. Los PNG alfa se obtuvieron
  localmente con `remove_chroma_key.py --auto-key border --soft-matte
  --transparent-threshold 12 --opaque-threshold 220 --despill`.
- Licencia prevista si se aceptan: `CC-BY-4.0`, como medio original del
  proyecto. La revisión humana debe confirmar esta atribución antes de publicar.

## Artefactos y cuarentena

Todas las fuentes miden 1537×1023 px. `generated/` conserva los resultados
opacos originales; `cutouts/` conserva los recortes alfa usados como entrada de
cuarentena.

| Pose | Recorte alfa | Registro de cuarentena | SHA-256 saneado |
| --- | --- | --- | --- |
| `idle` | `cutouts/gate-stalker-idle.png` | `quarantine/0ae9588a67fd9c72954970b118de2a67c69d985df5e21f5ee90375b3e3f33716/quarantine.json` | `0ae9588a67fd9c72954970b118de2a67c69d985df5e21f5ee90375b3e3f33716` |
| `attack` | `cutouts/gate-stalker-attack.png` | `quarantine/04e67ffb423161c88ea72ac9ef3eaa14ce86e8e214686811eef5364d471a58c9/quarantine.json` | `04e67ffb423161c88ea72ac9ef3eaa14ce86e8e214686811eef5364d471a58c9` |
| `hit` | `cutouts/gate-stalker-hit.png` | `quarantine/eba722558e24de467b61d7a9681be20cf6d26c24f52bf1adab8bbfee876191af/quarantine.json` | `eba722558e24de467b61d7a9681be20cf6d26c24f52bf1adab8bbfee876191af` |
| `defeated` | `cutouts/gate-stalker-defeated.png` | `quarantine/9a336d338447cc1313fedb527218c75607356cc20d65c4343ae71abbd18ba3bb/quarantine.json` | `9a336d338447cc1313fedb527218c75607356cc20d65c4343ae71abbd18ba3bb` |

Los cuatro registros siguen sin campo `review`. El saneado verificó PNG
estático, transparencia y dimensiones; no constituye aprobación editorial.

## Puerta exacta de revisión

Una persona debe inspeccionar cada pose sobre fondos claros y oscuros y
confirmar identidad coherente, anatomía, silueta, recorte, ausencia de halo
magenta, contenido no solicitado y procedencia/licencia. Solo entonces puede
adjuntar una revisión inmutable a cada registro:

```console
cargo run --locked -p tiamud-asset-pipeline --bin tiamud-assets -- review \
  --record <ruta-al-quarantine.json> \
  --reviewer "<persona>" \
  --reviewed-at "<fecha-RFC3339>"
```

Después de revisar las cuatro poses habrá que ejecutar el perfil
`entity-sprite` del pipeline para cada una y comprobar sus WebP/PNG publicados.
Solo esos artefactos publicados, no `generated/`, `cutouts/` ni `quarantine/`,
pueden incorporarse a manifiestos o WorldPack. Este paso queda deliberadamente
sin ejecutar hasta recibir la revisión humana explícita.
