# Autoría de mundos

Este es el punto de entrada vigente para crear contenido de Tiamud. El runtime
consume un **WorldPack v9** canónico. Los autores pueden mantenerlo como módulos
**AreaPack v8** estrictos bajo un **WorldProjectManifest v2** y componerlos sin
conexión. 6A publica un catálogo local y bundles inmutables; 6B los incorpora
bajo demanda; 6C puede sustituir solo la generación completa durante un corte
quiescente; 6D permite internamente retirar/rehidratar la misma revisión de una
unidad vacía. No hay reemplazo de revisión ni LRU operativo. Las áreas no son
servicios ni se despliegan de forma independiente.

Los tipos, validadores y pruebas Rust de `crates/world-pack` son la autoridad
exacta del esquema. Estas páginas explican el flujo soportado y enlazan el
diseño detallado sin duplicar cada campo.

## Por dónde empezar

1. Lee el [flujo en un único host](workflow.md).
2. Añade o modifica un [módulo de área](modular-worlds.md).
3. Define prototipos reutilizables e instancias iniciales durables con la
   [guía de esquema](../world-authoring.md#prototipos-e-instancias-de-objeto).
4. Define [progresión, habilidades y combate](progression-and-combat.md).
5. Compila y referencia [medios atmosféricos o sprites](media.md).
6. Ejecuta todas las [puertas de validación y publicación](validation-and-publishing.md).

El ejemplo revisado contiene:

- proyecto: [`worlds/lantern-square.project.json`](../../worlds/lantern-square.project.json);
- metadatos de release local:
  [`worlds/lantern-square.area-release.json`](../../worlds/lantern-square.area-release.json);
- módulos: [`worlds/areas/`](../../worlds/areas/); y
- artefacto canónico: [`worlds/lantern-square.world.json`](../../worlds/lantern-square.world.json).

## Ruta rápida

Ejecuta las herramientas sin conexión desde la raíz del repositorio. Los
ficheros de salida no se reemplazan en silencio.

```sh
mkdir -p build/worlds

cargo run --locked -p world-pack --bin tiamud-world -- compose \
  worlds/lantern-square.project.json \
  --output build/worlds/lantern-square.candidate.world.json

cargo run --locked -p world-pack --bin tiamud-world -- validate \
  build/worlds/lantern-square.candidate.world.json

cargo run --locked -p world-pack --bin tiamud-world -- test \
  build/worlds/lantern-square.candidate.world.json

cargo run --locked -p world-pack --bin tiamud-world -- preview \
  build/worlds/lantern-square.candidate.world.json \
  --room demo:lantern-square --format text

cargo run --locked -p world-pack --bin tiamud-world -- diff \
  worlds/lantern-square.world.json \
  build/worlds/lantern-square.candidate.world.json
```

No publiques solo porque un documento pueda decodificarse. Revisa antes prosa,
licencias, texto alternativo, estado oculto, balance y diff semántico.

## Catálogo, bundles y generaciones locales (fases 6A–6D)

6A no sustituye `compose` ni cambia WorldPack v9, AreaPack v8 o WorldProject
v2. Añade una segunda representación local y sin conexión:

- catálogo de hasta 1 MiB y 256 entradas;
- manifiesto de release de hasta 256 KiB; los tres sobres JSON son estrictos,
  rechazan claves duplicadas recursivas y limitan profundidad a 16;
- bundle canónico de hasta 16 MiB y suma de entradas también de 16 MiB;
- metadatos que ligan revisión, ownership, dependencias, portales y digests;
- `contentLicense=CC-BY-4.0` para mundo, prosa y medios, separado de
  `scriptLicense=MIT OR Apache-2.0` para Lua/software;
- `unsigned_local` explícito, que no es firma remota ni licencia de
  redistribución; y
- blobs inmutables por contenido con puntero local sustituido atómicamente.

`AreaPack.dependsOn` sigue siendo la declaración exhaustiva de autoría. El
catálogo v1 deriva `hardDependencies` solo de referencias cruzadas que no son
`RoomExit`; las salidas cruzadas son `portals` y no fijan residencia.
`softDependencies` está vacío y cada portal conserva su `sourceExit` cardinal.

La API Rust llama a los sobres `AreaReleaseManifest`, `AreaBundle` y
`AreaCatalog`. `build_area_release` construye un `VerifiedAreaRelease` y
`verify_area_release` contrasta catálogo, bundles y fallback monolítico. Cada
bundle incluye un `CompiledArea` estricto ligado por `compiledAreaSha256`.

```sh
cargo run --locked -p world-pack --bin tiamud-world -- area-publish \
  worlds/lantern-square.project.json \
  --release worlds/lantern-square.area-release.json \
  --registry build/area-registry
```

`area-publish` imprime la ruta content-addressed exacta. El fixture revisado
publica tres bundles y el catálogo
`e954be5b7e1d6918a1b87d5405d651f0fc52ee81996ab0c100cfcb1909ed15a6`:

```sh
cargo run --locked -p world-pack --bin tiamud-world -- area-verify \
  --catalog build/area-registry/area-catalogs/71b3f0f3493d91254d4ce1ed60c147e9f0f480ca204f62f05b3fd05d92e6e3f1/e954be5b7e1d6918a1b87d5405d651f0fc52ee81996ab0c100cfcb1909ed15a6.catalog.json \
  --registry build/area-registry
```

El registro usa
`area-catalogs/<world-key>/<catalog-digest>.catalog.json`,
`area-bundles/<world-key>/<area-key>/<bundle-digest>.area.json`,
`area-worlds/<world-key>/<world-digest>.world.json` y el puntero atómico
`area-active/<world-key>.json`. Las claves son SHA-256 de los IDs UTF-8 exactos.
El puntero contiene `schema`, `worldId`, `catalogSha256` y la ruta relativa
`catalog`. «Active» significa publicación editorial seleccionada, no residencia
de un bundle en el actor.

Los publicadores cooperantes se serializan mediante un `fd-lock` advisory por
mundo y releen la revisión activa dentro del lock. La verificación prevalida el
presupuesto agregado antes de E/S; un blob idéntico se reabre y resincroniza.
`area-verify` es read-only y solo acepta el path exacto publicado.

`unsigned_local` presupone un registro y host confiables. El lock no protege
frente a un proceso que lo ignore o pueda mutar el registro; un escritor hostil
y sus carreras TOCTOU quedan fuera.

6B incorpora bundles bajo demanda y los conserva residentes. 6C abre
`area-active` ante SIGHUP, verifica la release completa y construye en privado
la generación candidata. Solo sustituye la generación completa con cero
sesiones y cero leases; no expulsa a nadie. La compatibilidad exige gameplay,
objetos iniciales, NPC, botín, afijos, áreas, ownership e IDs estables idénticos
y migración persistente cero. Puede cambiar prosa, salidas y presentación/Lua
síncrono validado.

Durante el corte, el servidor retiene el mismo lock cooperativo del registro
que `area-publish`/`area-select` hasta resolver el CAS durable.

La generación activa vive en SQL v15; snapshot v8 y puntero avanzan por CAS en
una transacción. Para rollback selecciona un catálogo retenido y solicita el
corte:

```sh
cargo run --locked -p world-pack --bin tiamud-world -- area-select \
  --catalog <ruta-exacta-del-catalogo-retenido> \
  --registry build/area-registry
sudo systemctl kill --signal=HUP tiamud
```

Al actualizar desde 6B/v13 con recovery compartido o personajes, arranca v14
primero con el catálogo G0 exacto y
`TIAMUD_ACK_LEGACY_AREA_GENERATION=<sha256-exacto-G0>`. Verifica su liga durable
y retira la variable. La fila nueva nace vacía: introducir G1 en ese primer
arranque lo aceptaría como autoridad inicial sin comparar la compatibilidad con
G0. Copia backend y registro antes del cambio y publica o selecciona G1 solo
después de ligar G0.

Recovery vacío puede contener personajes. Sin liga generacional, recovery
compartido o cualquier personaje exige el ACK exacto de G0; cuentas solas sin
personajes/recovery de mundo pueden ligar sin ACK. Convertir un WorldPack
monolítico con autoridad existente exige migración offline y queda fuera de 6C.
Si un commit es ambiguo se relee SQL y, sin G0/G1 determinable, readiness queda
503. El ACK se rechaza en modo monolítico, con liga, sin recovery/personaje o
ante digest distinto.

6D añade retirada/rehidratación interna de la misma revisión con snapshot v8 y
SQL v15, pero permanece parcial. No existen reemplazo de revisión, migraciones
de estado, LRU, GC, red, compresión o firma remota. La puerta histórica 6C pasa
774 pruebas Rust —329 del servidor— y
la reauditoría 0 P1/0 P2. PostgreSQL 18 desechable pasa 20/20 pruebas del
adaptador y 7/7 enfocadas del servidor, incluida la CAS v14; el humo liga G0,
valida readiness/Web/Telnet, dos áreas residentes, `noop` y cierre limpio.
Consulta el
[registro de aceptación PostgreSQL v14/PWA](../project/log/2026-07-17-postgresql-v14-and-pwa-acceptance.md),
[ADR 0005](../decisions/0005-hot-area-lifecycle.md),
[ADR 0020](../decisions/0020-additive-local-area-loading.md) y
[ADR 0021](../decisions/0021-quiescent-area-generation-cutover.md), y consulta
[ADR 0022](../decisions/0022-reversible-area-unit-retirement.md) para 6D.

La puerta retenida 6A pasa 144/144 pruebas `world-pack`, incluidas 10/10 CLI,
y 707/707 Rust del workspace, además de clippy, formato/diff, MSRV Rust 1.88,
E2E, humo, cargo-deny y licencias/avisos. No debe atribuirse a 6C.

## Principios de autoría

- El texto y estado estructurado contienen todos los hechos jugables; los
  medios son presentación opcional.
- Los IDs namespaced estables son identidad de contenido. Renómbralos solo como
  una decisión explícita de migración.
- El contenido de objeto vive en `itemPrototypes`; cada unidad authored vive en
  `initialItemInstances` con una ubicación `room`, `npc_spawn`, `container` o
  `staged`. Las pilas fungibles se derivan: no publiques ni persistas ID o
  cantidad de pila, ni restaures los campos v3 `items`, `itemRules` o
  `lootItemIds`.
- WorldPack v9 y AreaPack v8 aceptan `transferable`, `character_bound` y
  `non_transferable`. El vínculo es estado runtime creado en la primera
  adquisición y persistido en snapshot v8, no un campo de autoría. WorldPack
  v4/AreaPack v3 y entradas más antiguas solo normalizan a `transferable`.
- La política de transferencia cubre todo el subárbol. Una bolsa transferible
  con un descendiente ligado/no transferible no puede darse o soltarse.
  `canGive` es ayuda de UI; el servidor revalida ownership, vínculo,
  proximidad, política social y ambos inventarios de forma atómica.
- Usa ubicación `npc_spawn` para botín fijo de una sola vez y
  `lootTables`/`lootSources` para drops repetibles. Garantizados preceden a los
  pools ponderados; `nothing` es explícito y cada spawn tiene una fuente. El
  peor caso fijo+repetible no supera 64 raíces.
- Tablas/fuentes nacen en WorldPack v6/AreaPack v5. Formatos anteriores
  normalizan a vacío y rechazan campos inyectados. El digest canónico de tabla
  se vuelve procedencia durable; IDs, pesos, cantidades y referencias son
  estrictos.
- Sellos y afijos nacen en WorldPack v7/AreaPack v6 y siguen vigentes. Un sello
  es único, consumible, `character_bound`, sin equipo/contenedor/efecto y
  referencia una tabla que garantiza 1..=16 unidades y como máximo 1.024 filas
  de odds. No es instancia inicial ni fuente NPC directa; procede de una tabla
  normal y no produce otro sello. `open` consume un sello ganado y `reward odds`
  muestra resultados authored sin revelar RNG, identidad o digests internos.
  Un pool elige sin sesgo un afijo como máximo para un item único no sellado.
  Apertura, RNG, contadores, creación, vínculo y capacidad confirman o revierten
  juntos. No son cajas pagadas, llaves o tiradas comprables; pity, need/greed,
  master-loot y contribución quedan fuera.
- WorldPack v9/AreaPack v8 conservan las habilidades de comunicación de v8/v7.
  La telepatía conocida tiene coste positivo, cooldown, un aliado online y hasta
  64 salas bloqueadas. No causa daño/curación/estados ni pertenece a sets NPC.
  Hay hasta 32; formatos anteriores rechazan el campo.
- WorldPack v9/AreaPack v8 añaden hasta 32 emotes cerrados. Comandos ASCII
  minúsculos únicos, targeting y plantillas determinan la forma exacta; solo se
  admiten `{actor}`/`{target}` y el peor caso renderizado cabe en 512 bytes.
  Formatos anteriores e importación normalizan a catálogo vacío.
- `WorldRuntimeSnapshot` v8 es la autoridad durable de ubicaciones, vínculo,
  reservas, procedencia, derechos, afijos y contador de apertura. SQLite y
  PostgreSQL v15 lo guardan como `TEXT` opaco; v1–v5 retienen 1 MiB y v6–v8,
  2 MiB. La conversión v6 añade afijos vacíos y contador uno. No hay tabla
  espejo de loot/escrow ni doble escritura.
- Títulos base/variante admiten 1..=128 bytes UTF-8 seguros; descripciones,
  1..=8.192. Se cuentan bytes, no caracteres.
- `entityMedia` de item apunta al prototipo; `roomPerception.hiddenItems`, a la
  instancia durable. No intercambies esas identidades.
- La visibilidad se resuelve en servidor. Nunca envíes a clientes un catálogo
  de salas, variantes, entidades o assets secretos.
- Personajes usan grants/requisitos acotados, no scripts arbitrarios de
  progresión.
- Lua tiene capacidades declaradas y cuotas duras; no evita el dominio.
- Conversión de assets e IA opcional permanecen fuera del juego.
- Una persona identificada revisa material importado, generado o subido antes
  de publicarlo.

## Referencias detalladas

- [Diseño de progresión v3](../design/character-progression-v3.md)
- [Diseño del compositor opcional](../design/scene-compositor-v1.md)
- [Decisión de personajes WorldPack v3](../decisions/0007-worldpack-v3-character-model.md)
- [Fundación prototipo/instancia](../decisions/0009-item-prototypes-instances-and-capacity.md)
- [Pilas derivadas y contenedores](../decisions/0010-derived-stacks-and-bounded-containers.md)
- [Give y políticas ejecutables](../decisions/0011-atomic-give-and-item-transfer-policies.md)
- [Reservas de trade](../decisions/0012-confirmed-trade-reservations.md)
- [Botín, derechos y procedencia](../decisions/0013-repeatable-loot-rights-and-provenance.md)
- [Sellos y afijos](../decisions/0014-earned-sealed-rewards-and-deterministic-affixes.md)
- [Telepatía authored](../decisions/0015-authored-online-telepathy.md)
- [Emotes authored](../decisions/0016-authored-room-emotes.md)
- [Compositor de escenas](../decisions/0008-optional-scene-compositor.md)
- [Ciclo de áreas](../decisions/0005-hot-area-lifecycle.md)
- [Carga aditiva 6B](../decisions/0020-additive-local-area-loading.md)
- [Corte quiescente 6C](../decisions/0021-quiescent-area-generation-cutover.md)
- [Retirada reversible 6D](../decisions/0022-reversible-area-unit-retirement.md)
- [Compilador de assets](../../crates/asset-pipeline/README.md)
- [Runtime Lua](../../crates/scripting/README.md)
- [IA editorial opcional](../../crates/editorial-ai/README.md)
- [Importador histórico Envy/Merc](../../tools/envy-import/README.md)

La [guía WorldPack v9/AreaPack v8](../world-authoring.md) es la referencia
compacta de esquema y flujo. Lantern Square contiene tres salas, diez
prototipos, nueve instancias iniciales, dos tablas, una fuente, un afijo, un
pool, una telepatía directa y un emote. El artefacto canónico de 53.567 bytes
tiene SHA-256
`9a75894a9e2e0c48d20257633f323d52bbe78b9db106de8c497f4784392b5e75`.
Mantiene `tiamud:road-token` y `tiamud:wolf-clasp` como transferibles y
demuestra 3C con `tiamud:rain-writ` no transferible en `demo:north-gate` y el
botín fijo ligado `tiamud:stormglass-talon` de `tiamud:gate-stalker-1`. Ese
spawn usa round-robin para un token garantizado, un sello ganado y una tirada:
60% clasp y 40% nada. El clasp normal y el talon del sello tiran además un
prefijo `storm-bitten` al 60%, demostrando que los afijos no son exclusivos de
sellos. El Wanderer recibe `tiamud:mind-whisper`; `demo:archive` demuestra su
bloqueo authored. `tiamud:smile` demuestra targeting opcional y plantillas por
perspectiva.
