# ADR 0032: misiones, mercancías y tiendas en WorldPack v10 / AreaPack v9

- Estado: aceptada — implementación parcial. Implementada como contenido del MUD (WorldPack v10,
  AreaPack v9, `tiamud-core`, cargador del servidor y datos de Lantern Square);
  fue aceptada por el propietario el 2026-10-16. Ver
  [`COMPLETION_MATRIX.md`](../project/COMPLETION_MATRIX.md).
  2026-10-16: aceptado por el propietario.
- Fecha: 2026-10-15
- Alcance: dominio del MUD (`tiamud-core`, `world-pack`, `tiamud-server`).
  Es la parte «MUD» que el plan de trabajo identifica como S8 de
  [ADR 0025](0025-narrative-ruleset-story-core.md); no toca `story-core`,
  StoryPack, sesiones de historia ni protocolo `story.*`.
- Relacionado: [ADR 0007](0007-worldpack-v3-character-model.md),
  [ADR 0014](0014-earned-sealed-rewards-and-deterministic-affixes.md),
  [ADR 0016](0016-authored-room-emotes.md),
  [ADR 0020](0020-additive-local-area-loading.md),
  [ADR 0021](0021-quiescent-area-generation-cutover.md). El número 0031 queda
  reservado para el ADR de integración del parser.

## Contexto

ADR 0025 señaló que `tiamud-core` contenía una misión cableada (`first-hunt`)
y la tabla de mercancías de la tienda (`commodity_specs`) también estaba en
código: dos mercancías, precios fijos, stock inicial 20 y máximo 100, y la
tienda ligada implícitamente a la sala inicial. Ningún autor podía añadir otra
misión o tienda sin cambiar Rust, y el validador de snapshots aceptaba solo el
ID `first-hunt`.

## Decisión

1. **Formato**. WorldPack v10 añade bajo `gameplay` tres colecciones cerradas
   y acotadas, con `deny_unknown_fields` en cada DTO: `commodities`, `quests`
   y `shops`. AreaPack v9 las declara en su raíz; la composición registra sus
   IDs en el espacio global y la propiedad del catálogo 6A las incluye como
   campos opcionales que no alteran catálogos anteriores.
2. **Misiones**: ID durable, nombre/resumen `LocalizedText`, alias de comando,
   repetibilidad `once|repeatable`, 1..=16 etapas ordenadas de 1..=8 objetivos
   (`defeat_npc` con prototipo opcional, `collect_item`, `visit_room`,
   `talk_to_npc`) y recompensas de monedas/experiencia. Los prerrequisitos
   usan un subconjunto cerrado de `Condition` (`all`, `any`, `not`,
   comparaciones de `actor.level`, `contains` sobre `actor.flags` y
   `actor.quest_states`) que el núcleo evalúa solo con estado durable del
   actor.
3. **Tiendas**: una por sala, guardián opcional (spawn vivo en esa sala),
   1..=32 entradas de stock con `stockId` durable único en el mundo, precios
   con `sellPrice <= buyPrice`, stock máximo 1..=10.000 y reposición
   `none|interval` determinista por tick lógico.
4. **Compatibilidad sin cambiar el comportamiento existente**. Normalizar un
   WorldPack v2–v9 inyecta el contenido legado implícito (misión, dos
   mercancías y `lantern-market` en `startRoomId`), idéntico al que el código
   tenía cableado. `World::from_definition` conserva ese mismo contenido
   (`AdventureContent::legacy`) como valor por defecto para quien construya el
   mundo sin WorldPack; el servidor siempre lo sustituye por el del pack. Un
   catálogo publicado antes de AreaPack v9 solo puede llevar contenido de
   aventura sin propietario si es exactamente ese legado.
5. **Estado durable**. El progreso por personaje sigue en
   `CharacterGameplaySnapshot.quests` con la clave del ID durable; un mapa
   opcional `questStages` (etapa, contadores por objetivo, completados) solo
   aparece cuando hace falta, de modo que el JSON existente no cambia. El
   stock sigue en `AdventureRuntimeSnapshot.shopStock`, ahora con clave
   `stockId`; para Lantern Square las claves son las mismas (`tonic`,
   `trail-ration`). **No se sube la versión de `WorldRuntimeSnapshot`** (v9)
   ni el esquema SQL (v16): ni la forma del snapshot ni la de las fichas
   cambian para el mundo existente. La validación semántica sin pack comprueba
   forma y cotas; la restauración con pack exige exactamente las entradas
   configuradas y su máximo, y la validación de fichas rechaza misiones
   desconocidas, etapas fuera de rango, contadores incoherentes o completados
   de una misión `once`.
6. **Atomicidad**. Aceptar, avanzar y reclamar ocurren dentro de la acción o
   del tick que ya delimita el checkpoint del actor. El avance por estado
   (visitar, reunir) se evalúa tras cada acción con éxito, en la misma
   transición. Una derrota o conversación avanza como mucho una etapa.
   Reclamar suma monedas y experiencia con aritmética comprobada y marca
   `completed` en un solo paso; un segundo intento falla.
7. **Modo catálogo**. Igual que progresión y emotes, el catálogo de aventura se
   fija globalmente desde el respaldo monolítico verificado; no se instala por
   área ni crea dependencias duras. Cambiarlo altera `gameplay`, así que un
   corte 6C lo rechaza.

## Consecuencias

- La salida Web/Telnet/GMCP del mundo existente es idéntica: textos, IDs,
  comandos, vistas y snapshots se conservan y hay pruebas doradas.
- `talk <npc>` / `talk to <npc>` es un comando de texto nuevo que entra por el
  mismo parser en navegador y Telnet; no añade mensajes de protocolo.
- Las vistas proyectan como máximo 16 misiones por escena para respetar el
  marco de 16 KiB.

## Límites

No implementado: stock de prototipos de objeto (las tiendas venden solo
mercancías abstractas), recompensas selladas o concesiones de progresión como
recompensa de misión, consumo de objetos al reclamar, locale por jugador
(se resuelve `en`), crédito por derrotas fuera del tick lógico y herramientas
de autoría o importación avanzadas de S8.

## Alternativas descartadas

- **Subir el snapshot a v10** con un mapa de stock por tienda: innecesario,
  porque un `stockId` durable conserva la forma y las claves existentes.
- **Evaluar `Condition` completo en el núcleo**: obligaría a `tiamud-core` a
  depender de `world-pack` o a duplicar tiempo/clima del servidor; el
  subconjunto basta para prerrequisitos de personaje.
- **Normalizar los WorldPack antiguos sin contenido**: quitaría la misión y la
  tienda a mundos revisados existentes, un cambio de juego silencioso.
