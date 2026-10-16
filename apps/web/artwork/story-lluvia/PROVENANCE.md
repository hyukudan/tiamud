# Arte de muestra de `lluvia-de-neon`

Estado: **publicado en `apps/web/public/assets/` como placeholder original**,
mapeado en el mapa revisado de medios de historia
[`crates/story-core/fixtures/story-media.json`](../../../../crates/story-core/fixtures/story-media.json).

## Procedencia

- Fecha: 2026-10-15.
- Origen: dibujado solo con código por [`generate.py`](generate.py)
  (degradados, rectángulos, elipses y polígonos con Pillow). No se usó ninguna
  imagen, fotografía, tipografía, modelo generativo ni obra de terceros.
- Autoría y licencia: Tiamud project contributors, `CC-BY-4.0`, como el resto
  de medios originales del proyecto.
- Reproducción: `python3 -I apps/web/artwork/story-lluvia/generate.py
  apps/web/artwork/story-lluvia/sources` y después
  `bash apps/web/artwork/story-lluvia/build.sh` desde la raíz del repositorio
  (pipeline `tiamud-assets build`, perfil atmosphere, variantes AVIF/WebP/JPEG
  de 640 y 1280 px).

| Id de StoryPack | Fuente | Bundle publicado |
| --- | --- | --- |
| `fondo-oficina` | `sources/fondo-oficina.png` (1280×560) | `fb9c51c757db6b8908ca3a7228d31e1e7270d8d857036c1c2fa4b52c8ac131d2` |
| `fondo-club` | `sources/fondo-club.png` (1280×560) | `7f2a2d0796ad2b0b7ec90b8484213f18390d5784898f73c0967073b3fdc5eb98` |
| `retrato-lola-neutral` | `sources/retrato-lola-neutral.png` (1280×1280) | `1397e02d05d58ed08bc4c75411dc5d9c8a1dd89246cbb476fd109ed23247e1fc` |
| `retrato-damaso-neutral` | `sources/retrato-damaso-neutral.png` (1280×1280) | `8432f72a44714fd24b61af713105a731edb0002022ba313632ba7acb914c577c` |

`fondo-callejon`, `fondo-amanecer` y las demás expresiones quedan sin mapear a
propósito: demuestran que un id sin entrada no produce imagen y que el texto
conserva todo lo jugable.

## Revisión

El campo `humanReviewedAt` de los manifiestos (2026-10-15T00:00:00Z) marca el
punto de construcción revisado por el agente que generó el arte bajo encargo
del propietario: se comprobaron visualmente las cuatro fuentes (siluetas y
formas geométricas sin texto, logotipos ni contenido sensible) y el texto
alternativo es/en. **La aprobación editorial humana del propietario sigue
pendiente**; hasta entonces trátese como arte de muestra sustituible. La
prueba `story_sessions::media_tests::the_shipped_media_map_points_at_reviewed_pipeline_bundles`
verifica que cada entrada apunta a un manifiesto schema 2 `atmosphere` con
licencia `CC-BY-4.0` y variantes cuyos bytes y SHA-256 coinciden.
