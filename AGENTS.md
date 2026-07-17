# Guía para agentes de Tiamud

Este archivo se aplica a todo el repositorio. Es el contrato de trabajo para
agentes de programación y contribuidores. Debe permanecer alineado con el
código actual, las decisiones aceptadas y la evidencia retenida.

## Lectura inicial

Antes de un cambio material, lee en este orden:

1. [`docs/project/PROJECT_MEMORY.md`](docs/project/PROJECT_MEMORY.md), contexto
   técnico durable y notas de continuación.
2. [`docs/project/COMPLETION_MATRIX.md`](docs/project/COMPLETION_MATRIX.md),
   estado autoritativo de implementación y aceptación.
3. Los ADR pertinentes en [`docs/decisions/`](docs/decisions/).
4. [`docs/spec-v0.3.md`](docs/spec-v0.3.md), el diseño relevante y el código.
5. Los registros fechados de [`docs/project/log/`](docs/project/log/) solo si
   importan comandos o evidencia histórica exactos.

Precedencia de fuentes:

- los ADR aceptados definen decisiones arquitectónicas;
- el código y las pruebas enfocadas demuestran comportamiento;
- la matriz determina si un requisito está completo o parcial;
- la memoria es el resumen durable y punto de relevo;
- los registros fechados describen la entrega de aquel momento y pueden quedar
  superados;
- `docs/spec-v0.2.md` es la especificación histórica en castellano, no el
  contrato vigente de personajes, medios o sistemas sociales.

No deduzcas implementación porque un ADR o una hoja de ruta describa algo. No
cambies una decisión aceptada en silencio: añade o sustituye un ADR y actualiza
matriz, memoria y documentación pública a la vez.

## Identidad del producto y repositorio

- El nombre definitivo es **Tiamud**. No reintroduzcas un título provisional ni
  el antiguo nombre del repositorio como identidad.
- El repositorio canónico es `hyukudan/tiamud`; `main` es la rama predeterminada
  y la única de desarrollo activa.
- `envy-import` es solo la herramienta sin conexión para compatibilidad con
  datos EnvyMUD/Merc. Tiamud es una implementación limpia, no un fork.
- El proyecto está preparado bajo licencias de código abierto y en prelanzamiento
  `0.1.0`; el repositorio canónico permanece privado hasta una decisión explícita
  de visibilidad. No existe despliegue público de producción ni versión estable
  del protocolo.
- Software, pruebas, configuración, scripts y binarios: `MIT OR Apache-2.0`.
  Prosa, documentación, mundos y medios originales: `CC-BY-4.0`. Conserva la
  frontera y la procedencia de cada recurso.
- Nunca supongas que un input histórico o de terceros puede redistribuirse.
  Compatibilidad no es licencia.

## Baseline implementado

### Entorno de ejecución y persistencia

- Workspace Rust 2024, toolchain 1.93.1 y MSRV 1.88.
- Un proceso Axum/Tokio `tiamud-server` posee un actor de mundo con escritor
  único e incluye el cliente web revisado y Lantern Square.
- Navegador y Telnet producen la misma `tiamud_core::PlayerAction` y entran por
  la misma autorización y dominio.
- El actor serializa mutaciones. Un commit durable fallido restaura exactamente
  el checkpoint en memoria; no debilites esa frontera.
- Mundo y conversación usan buzones acotados de 256 entradas por sesión,
  encaminados por `SessionId`. Conserva el fan-out O(S), recuperación por lag y
  limpieza segura ante cancelación.
- PostgreSQL esquema v15 es el perfil recomendado para un host público nuevo.
  SQLite esquema v15 es la ruta local/predeterminada sin servicios. Se abre un
  único sistema autoritativo; no hay doble escritura.
- La última aceptación PostgreSQL integral retenida corresponde a v14 y usa
  PostgreSQL 18 desechable: 20/20 pruebas
  `--all-targets` del adaptador y 7/7 pruebas enfocadas del servidor, incluida la
  CAS de generación. El humo confirma esquema 14, liga G0, readiness, Web,
  Telnet, dos áreas residentes, `noop` por SIGHUP y cierre limpio. Esto no
  demuestra por sí solo el esquema v15 ni sustituye TLS, backup/restore,
  monitorización o recuperación del operador.
- `tiamud-migrate-postgres` traslada sin conexión una instantánea SQLite actual
  a PostgreSQL vacío en una transacción, incluidas suscripciones, perfiles,
  capacidades, reportes resueltos, auditoría de staff, mensajes/cuotas de
  correo, tablones, cuotas, evidencia denunciada y la identidad durable de la
  generación de áreas.
- Los datos consultables son relacionales. Las instantáneas JSON acotadas se
  guardan como `TEXT` opaco en ambos adaptadores; no uses `jsonb` sin necesidad
  medida y ADR.
- `WorldRuntimeSnapshot` v8 es la autoridad durable global. Conserva el grafo de
  instancias, reservas de trade, procedencia y derechos de botín, afijos por
  instancia y contador de aperturas de v7 y añade un manifiesto acotado
  `Resident|Retired` para las áreas de una generación de catálogo. SQLite y
  PostgreSQL la confirman con personajes/eventos en su transacción existente;
  ambos adaptadores usan esquema v15 y no hay tabla espejo ni custodia SQL de
  trade o botín. DTO exactos v1-v8 se seleccionan por `snapshotVersion`;
  snapshots v1-v5 conservan su límite legado de 1 MiB y v6-v7 admiten hasta
  2 MiB; v8 mantiene ese máximo. El DTO v6 estricto rechaza campos v7 y
  normaliza a afijos vacíos y
  `nextRewardOpenId: 1` sin inventar procedencia.
  v1-v3 se convierten a vínculo `unbound` y un campo
  `binding` inyectado en v3 se rechaza; v4 crea `nextTradeId: 1` y `trades`
  vacío. La lectura falla cerrada ante campos de otra versión, referencias,
  grafo, vínculo, trade, reserva, política o contador inválidos. Al
  restaurar un snapshot anterior se conservan las nuevas instancias iniciales
  del WorldPack y se superponen las ubicaciones durables; una instancia creada
  en runtime que no esté en el snapshot no reaparece.
- El esquema v15 añade `world_area_checkpoints`. Cada fila liga mundo,
  generación, unidad, revisiones, epoch, versión, tick, JSON estricto acotado y
  SHA-256. Retiro y rehidratación confirman por CAS checkpoint + snapshot v8 o
  consumen ambos atómicamente. El mover SQLite→PostgreSQL valida y copia esas
  filas; no son otra autoridad de trade, botín ni personajes.
- Cada snapshot registra la secuencia cubierta y conserva atómicamente solo el
  sufijo diagnóstico cubierto de 10.000 secuencias.
- Autenticación registrada compartida entre web y Telnet. Trabajo Argon2id,
  vidas de sesión, conexiones y colas están acotados. Solo se almacenan
  resúmenes SHA-256 de credenciales bearer.

### Mundo, personajes y juego

- WorldPack v9 es canónico; AreaPack v8 es la autoría modular nativa y
  WorldProject permanece en v2. WorldPack v8/v7/v6/v5/v4/v3/v2,
  AreaPack v7/v6/v5/v4/v3/v2/v1 y Envy/Merc solo entran por
  normalización/importación.
- El arranque elige una fuente de mundo. `TIAMUD_WORLD_PACK` selecciona un
  WorldPack monolítico externo revisado; sin variables de mundo se usa el
  embebido. Como alternativa 6B, `TIAMUD_AREA_REGISTRY` y
  `TIAMUD_AREA_CATALOG` deben aparecer juntas, son excluyentes con
  `TIAMUD_WORLD_PACK` y fijan una generación local content-addressed antes de
  abrir listeners.
- `tiamud-world compose` realiza composición modular determinista sin conexión.
  La fase 6A está **implementada** y añade una ruta local separada de catálogo y
  bundles: manifiesto de release de hasta 256 KiB, catálogo de hasta 1 MiB/256
  entradas, bundles JSON canónicos de hasta 16 MiB y suma de entradas de hasta
  16 MiB; los tres sobres limitan JSON a profundidad 16 y rechazan claves
  duplicadas recursivamente. Los metadatos registran área/revisión,
  propiedad/dependencias/portales/digests, confianza
  `unsigned_local`, `contentLicense=CC-BY-4.0` para mundo/prosa/medios,
  `scriptLicense=MIT OR Apache-2.0` para Lua/software, blobs inmutables y
  puntero atómico. No deduzcas por 6A la carga runtime: esa frontera pertenece
  a 6B. La sustitución quiescente completa pertenece a 6C; reemplazo por área,
  descarga, LRU, red, compresión y firma remota siguen ausentes.
- En 6A, `AreaPack.dependsOn` sigue siendo declaración editorial exhaustiva.
  `hardDependencies` deriva solo de referencias cruzadas no-`RoomExit`; cada
  `RoomExit` cruzado se proyecta como `portals` y no fija residencia.
  `softDependencies` está vacío en el catálogo v1.
- El bundle incluye un `CompiledArea` estricto y su `compiledAreaSha256`; cada
  portal conserva `sourceExit` cardinal. El CLI exacto es `tiamud-world
  area-publish <project> --release <json> --registry <dir>` y `area-verify
  --catalog <path> --registry <dir>`. El verificador exige la ruta
  content-addressed exacta dentro de `area-catalogs/`. El puntero
  `area-active/<sha256(worldId)>.json` selecciona una publicación editorial
  local, no una revisión cargada por el actor.
- `area-publish` serializa publicadores cooperantes mediante un lock advisory
  `fd-lock` por mundo, prevalida conteo/suma antes de E/S, resincroniza un blob
  idéntico existente y publica el path exacto por stdout. `area-verify` es
  read-only. `unsigned_local` exige registro local en host confiable: el lock
  no protege frente a un proceso que lo ignore ni frente a un escritor hostil;
  esas carreras TOCTOU quedan fuera de 6A.
- La fase 6B del [ADR 0020](docs/decisions/0020-additive-local-area-loading.md)
  está **implementada** como carga local aditiva bajo demanda. La identidad de
  generación es el SHA-256 de los bytes canónicos exactos del catálogo y queda
  fijada hasta apagar; cambios posteriores de `area-active` se ignoran. Un
  arranque sin snapshot ni ledger carga el área de `startRoomId` más su cierre
  duro; cualquier autoridad durable previa obliga a cargar toda la generación
  antes de restore/replay. Los portales no son dependencias duras: quedan como
  `DeferredExit` hasta una entrada real.
- El coordinador 6B está fuera del actor, coalesce por
  `(catalogSha256, areaId)` y limita dos preparaciones activas, ocho pendientes,
  cinco segundos totales y un cierre duro de 32 áreas. E/S, digest, JSON,
  conversión y Lua se preparan fuera del actor. Este instala el lote bajo un
  checkpoint conjunto de `World`, `RuntimeContent` y residencia, y reintenta la
  acción original una vez. El resultado interno nunca cruza Web/Telnet/GMCP;
  todo fallo público usa un destino no disponible genérico.
- Una instalación 6B puramente estática no escribe snapshot. Si crea estado
  dinámico inicial cubierto por snapshot v8, lo confirma mediante la transacción
  durable existente antes de publicar. Las métricas no etiquetan `areaId`,
  room ID, revisión, digest o generación. Todas las áreas incorporadas quedan
  residentes hasta apagar: 6B **no** implementa sustitución, coexistencia de
  generaciones, drenaje, descarga, LRU, red, compresión ni firma remota.
- La fase 6C mínima de
  [ADR 0021](docs/decisions/0021-quiescent-area-generation-cutover.md) está
  implementada y ha cerrado su puerta funcional. Sustituye una generación completa,
  nunca una sola área: SIGHUP local resuelve `area-active`, verifica release y
  staging completo fuera del actor y aplica una compatibilidad sin migración.
  Exige reglas de juego, objetos iniciales, NPC/botín/afijos, áreas, ownership e
  IDs estables idénticos; puede cambiar prosa, salidas y presentación/scripts
  síncronos validados. La candidata no comparte gauges con la activa; solo el
  swap confirmado publica sus métricas.
- 6C cierra admisiones y exige cero sesiones y cero leases; si no, devuelve
  `busy` sin expulsar ni mover jugadores. En el baseline actual exige que todas
  las áreas estén residentes y confirma `WorldRuntimeSnapshot` v8 más la
  generación mediante CAS. `world_area_generation`, introducida por el esquema
  v14 y conservada en v15, no guarda rutas ni estado de juego duplicado. El
  swap en memoria. El primer arranque liga la generación; después recovery abre
  el digest durable aunque `area-active` haya cambiado. Una activación normal
  exige revisiones crecientes; `tiamud-world area-select --catalog <path>
  --registry <dir>` seguido de SIGHUP hace explícito el rollback.
- Un fallo definitivo antes del commit conserva la generación anterior. Un
  resultado de commit ambiguo obliga a releer SQL: publica la nueva si quedó
  durable, reabre la anterior si no y, ante otro valor o error, permanece en
  `Committing`, detiene el actor y deja readiness 503 hasta reinicio/recuperación
  durable. Nunca supongas rollback ante un commit ambiguo.
- Al migrar una instalación 6B/v13 con recovery compartido o personajes, el
  primer arranque v14 debe conservar el G0 exacto. La fila nueva nace vacía y la liga inicial no puede
  comparar una identidad anterior inexistente: cambiar binario/esquema y
  catálogo a G1 a la vez elude compatibilidad/CAS. Exige copia de backend y
  registro, el mismo catálogo G0 y
  `TIAMUD_ACK_LEGACY_AREA_GENERATION=<sha256-exacto-G0>` solo durante esa liga;
  verifica y retira la variable antes de publicar/seleccionar G1 + SIGHUP.
- `StartupWorldRecovery::Empty` no prueba ausencia de autoridad: puede haber
  personajes sin snapshot/ledger. Sin liga generacional, recovery compartido o
  cualquier personaje durable exige el ACK exacto del G0 6B. Cuentas sin
  personajes ni recovery de mundo pueden ligar sin ACK. Convertir una
  instalación monolítica con autoridad de mundo/personaje a catálogo queda
  fuera de 6C y requiere migración offline. La aceptación se rechaza en modo
  monolítico, con liga existente, sin recovery/personaje o ante digest distinto.
  `CharacterStore::has_durable_character_authority` mantiene la misma consulta
  acotada en SQLite, PostgreSQL, el wrapper medido y memoria de pruebas.
- Durante un corte, el runtime retiene el mismo lock cooperativo del registro que
  `area-publish`/`area-select` desde la selección estable hasta resolver el CAS;
  no debilites esa frontera entre puntero editorial y autoridad durable. En
  Solaris usa el mismo locking de registro `fcntl` compatible con `fd-lock`.
- 6C **no** añade jugadores durante drenaje, migraciones de estado, reemplazo o
  sustitución de revisión por área, LRU, GC, red, compresión o firma remota. La
  matriz solo permanece completa mientras el registro 6C conserve su evidencia.
- La fase 6D del [ADR 0022](docs/decisions/0022-reversible-area-unit-retirement.md)
  está **parcialmente implementada**. El código define SCC y cierre inverso
  deterministas, snapshot v8, checkpoint dinámico, pins del dominio, retirada
  y rehidratación de la misma revisión, desmontaje/reinstalación exactos de
  `RuntimeContent` y transacciones CAS SQLite/PostgreSQL v15. La ruta interna
  del actor rehidrata una unidad retirada antes de movimiento o entrada y el
  arranque valida manifiesto y checkpoints antes de readiness.
- No presentes 6D como política operativa terminada: no hay presupuesto/LRU,
  histéresis, superficie de operador o protocolos de juego, sustitución de
  revisión individual, coordinación multiproceso, E2E específico de ACK perdido
  6D, medición RSS aceptada ni gate integral final. Una unidad solo se
  retira vacía, sin pins y dentro de la misma generación/revisión.
- La autoría mide UTF-8 por bytes: título de sala 1..=128 y descripción
  1..=8.192; rechaza controles inseguros y normaliza entradas legacy antes de
  validar. No subas estos topes para hacer caber prosa generada.
- El modelo de personaje v3 funciona de extremo a extremo dentro de WorldPack
  v9: linaje, trasfondo opcional, clase
  base/avanzada, XP global, clase activa, historial, reservas, talentos con
  rango y promoción atómica.
- `archetype` no es identidad canónica. Normalízalo en la frontera; no crees un
  modelo heredado paralelo.
- Concesiones y requisitos permanentes usan vocabularios cerrados y acotados.
  Aumentar capacidad no cura/recarga; un recurso nuevo empieza vacío.
- Habilidades marciales, mágicas y de utilidad comparten definición acotada con
  costes, cooldown, objetivos, daño/curación, resistencia y estados. Las
  habilidades del jugador respetan un cooldown global de un ciclo lógico.
- Los NPC eligen habilidades de forma determinista y tienen ataque básico de
  respaldo. No hay IA generativa en ejecución.
- Objetos/equipo, combate, derrota/recuperación, botín fijo y repetible, tiempo, clima,
  misiones, grupos, economía, conversación y moderación persisten en ambos
  adaptadores. Los objetos separan `itemPrototypes` de instancias durables;
  cada personaje tiene capacidad autoritativa por slots/peso y rareza cerrada
  `common|uncommon|rare|epic|legendary` más `itemLevel` textual en Web, Telnet
  y GMCP. Las unidades únicas reciben IDs opacos por instancia; una pila usa el
  selector opaco de su primer miembro visible ordenado. Todos se derivan de una
  sal criptográficamente aleatoria nueva por sesión. Telnet muestra la
  referencia opaca y permite usarla en comandos. En autoría,
  `entityMedia(kind=item)` referencia el prototipo reusable, mientras
  `roomPerception.hiddenItems` referencia la instancia concreta. Las pilas
  fungibles son vistas derivadas por ubicación/prototipo/vínculo/afijo: no
  persisten cantidad ni identidad de pila. Un afijo durable por instancia
  añade un prefijo o sufijo y deltas acotados de ataque/curación; su copia y
  digests se validan al restaurar, pero los IDs/digests no se proyectan. Los
  contenedores son instancias únicas con
  slots/peso propios, peso recursivo, profundidad máxima cuatro y grafo
  validado. `take`/`drop`/`put`/`get` y `give` aceptan 1..=256 unidades y
  `contents` pagina 20 grupos; Web, Telnet y GMCP reciben cantidad, slots, peso
  total, capacidad, política de transferencia y estado de vínculo mediante
  referencias opacas.
- WorldPack v9 y AreaPack v8 aceptan el vocabulario cerrado `transferable`,
  `character_bound` y `non_transferable`, igual que WorldPack v6/v5 y AreaPack
  v5/v4;
  WorldPack v4/AreaPack v3 y anteriores solo normalizan `transferable`. La primera adquisición liga
  recursivamente un `character_bound`. Mover dentro del grafo del mismo dueño
  está permitido, pero `drop` o un cambio de dueño exige que toda la raíz y sus
  descendientes sean `transferable`; `non_transferable` no guarda dueño ligado
  pero tampoco puede abandonar al adquirente.
- `give [cantidad] <objeto> to <personaje>` mueve solo raíces portadas a otro
  personaje conectado de la misma sala. Acepta selector opaco o nombre exacto,
  rechaza al propio emisor, prevalida capacidad de ambos agregados, respeta
  ignore del receptor y block bilateral y confirma un único `ItemsGiven` antes
  de proyectar. La entrega offline/remota sigue ausente.
- `trade` implementa intercambio bilateral confirmado entre dos personajes
  conectados, en la misma sala, fuera de combate y permitidos por la política
  social. El escrow es una reserva durable: objetos y monedas no cambian de
  autoridad hasta que dos identidades confirman la misma revisión y digest.
  Hay como máximo 64 trades, uno por personaje, 8 líneas por participante y
  128 raíces transferibles entre ambos; solicitud y negociación caducan a 20 y
  120 ticks. Cualquier cambio material invalida ambas confirmaciones. La
  liquidación prospectiva mueve ambos lotes y saldos simultáneamente en un
  único commit, y un reinicio valida participantes y cancela reservas antes de
  readiness. Web, Telnet y `Char.Trade 1` GMCP comparten este dominio.
- El arranque enumera de forma acotada la unión de personajes que poseen raíces
  y todos los `CharacterId` ligados, incluso en instancias consumidas. Carga
  temporalmente sus agregados y valida existencia, vínculo, equipo, grafo y
  capacidad, incluidos los desconectados. Esos agregados no quedan retenidos
  como personajes conectados. Una incoherencia bloquea listeners.
- WorldPack v6 añadió tablas enteras de botín y fuentes por aparición de NPC;
  WorldPack v7/AreaPack v6 añaden recompensas selladas y afijos.
  Cada derrota resuelve una vez el botín fijo y repetible, con rejection
  sampling sin sesgo, procedencia durable y máximo combinado de 64 raíces. La
  capacidad puede suprimir un drop sin bloquear la derrota ni permitir reroll.
  `party_round_robin` congela una cohorte de hasta seis personajes y `loot pass`
  cede el derecho sin renovar el plazo; la reserva es exclusiva 20 ticks,
  pública después y desaparece al tick 120. Reiniciar cancela encuentros
  abiertos antes de readiness, pero conserva intactos los derechos ya creados.
  Un sello solo se obtiene como botín NPC, es raíz portada, única, consumible y
  `character_bound`, y no puede equipar, contener, atacar, curar ni aplicar
  efectos. `open <sello>` simula RNG, IDs, afijo, vínculo, capacidad y hasta 16
  recompensas antes de consumir y confirmar todo en un commit; cualquier fallo
  restaura el checkpoint exacto. `reward odds <sello> [página]` expone
  fracciones exactas en páginas de 20 sin IDs/digests internos. Web, Telnet y
  `Char.Reward 1` GMCP comparten acciones y proyección. Need/greed,
  master-loot y contribución son fases posteriores; cajas, llaves, moneda
  premium, pity o tiradas pagadas no pertenecen al baseline.
- Los grupos admiten hasta seis miembros y soportan
  create/invite/accept/revoke/kick/transfer/disband/leave/status y `party say`
  (`gsay`/`gtell`). No hay follow/assist/rescue, selección general de aliados,
  XP/crédito de misión compartidos ni reparto avanzado de botín.
- La membresía permanece al desconectar y el snapshot muestra miembros offline,
  pero accept/revoke/kick/transfer resuelven un nombre conectado. No afirmes
  administración dirigida de un líder/invitado/miembro desconectado.
- La conversación incluye `say` local, `tell`, `reply` en memoria mientras el
  destinatario siga conectado, chat de grupo y canales `general`, `newbie` y
  `staff`. Las suscripciones son durables desde v13 y se conservan en el esquema
  v15 actual; `staff` requiere perfil
  `moderator`, `administrator` o `implementor` al unirse, enviar y recibir;
  `helper` y `builder` no acceden. WorldPack v9/AreaPack v8 conservan una
  comunicación `telepathy/direct_online` en habilidades `spell|utility`
  `single_ally`: exige al menos un coste, cooldown positivo y conocimiento;
  no admite daño/curación/estados ni uso por NPC, y puede bloquear hasta 64
  salas. `telepathy <ability-id> <personaje> <mensaje>` consume recurso y
  cooldowns en un commit antes de entregar; ignore/block, estado o sala del
  receptor suprimen la entrega sin revelarlo. No actualiza `reply`, no guarda
  cuerpo/destino y comparte mute/bucket. Web, Telnet y `Comm.Channel.Text`
  GMCP comparten política. El correo offline es una superficie separada:
  `mail list/read/delete/send/report`, 64 mensajes y 128 KiB por buzón,
  caducidad de 30 días, cuotas por cuenta 20/h y 50/día, aceptación opaca ante
  ignore/block/capacidad y evidencia denunciada durante 90 días. Web publica
  `mail.list`/`mail.message`; Telnet y `Tiamud.Mail 1` son equivalentes. Los
  tablones públicos `general` y `newbie` son otra superficie pull-only:
  `board catalog/list/read/post/delete/report`, diez entradas por página,
  retención de 90 días, cuota de cuenta 5/h y 20/día, 64 publicaciones vivas
  por cuenta, 1.000/4 MiB por tablón y 2.000/8 MiB globales. Borrar exige el
  mismo personaje autor; una cuenta distinta puede crear la única denuncia
  global de un post. La evidencia dura 90 días y staff la resuelve mediante
  `reports.view`/`reports.resolve`. Web `board.catalog/list/entry`, texto
  Telnet y `Tiamud.Board 1` son equivalentes; no hay fan-out, notificación,
  edición, hilos, búsqueda ni vínculo con sala/AreaPack. La línea Telnet máxima
  es 8 KiB y el detalle GMCP que crece por escape JSON conserva resumen,
  prefijo UTF-8 del cuerpo y metadatos de overflow bajo el mismo tope; texto
  conserva el cuerpo completo. Web rechaza claves JSON duplicadas a cualquier
  profundidad y solo aplica post/delete/report tras `action.result`
  correlacionado, conservando borradores en fallo. La fase 5E implementa
  `yell` de radio uno y `shout` de radio tres: BFS
  dirigido determinista sobre la sala origen, sin atravesar salidas secretas,
  con máximo de 259 salas y sin proyectar dirección o distancia. Texto y Web
  aceptan mensajes de 1..=512 bytes; Web usa `social.yell`/`social.shout`
  exactos, Telnet conserva los comandos y GMCP reutiliza `Comm.Channel.Text`.
  Mute y el bucket común bloquean emisión; ignore, block bilateral y
  `condition.deaf` suprimen por receptor `say`/`yell`/`shout` sin impedir que
  el sordo hable ni cambiar la confirmación del actor. `OnTalk` se ejecuta una
  vez en origen y la entrega empieza post-commit. No hay historial general
  durable, persistencia de voz ni telepatía de grupo/offline.
  WorldPack v9/AreaPack v8 añaden hasta
  32 emotes locales authored: `emotes` lista y `emote <comando> [personaje]`
  renderiza plantillas cerradas por perspectiva, sin texto libre, persistencia
  ni reevaluación de placeholders insertados. El objetivo es otro personaje
  conectado de la misma sala; ignore/block suprime por receptor y el commit de
  `OnTalk` precede a Web/Telnet/GMCP.
- La autoridad ejecutable de staff usa capacidades cerradas y nunca procede de
  XP, nivel, clase, talentos o WorldPack. Los perfiles `helper`, `builder`,
  `moderator`, `administrator` e `implementor` siembran bundles revisados y
  admiten un `lore_title` opcional que no autoriza nada. Las doce capacidades
  son `reports.view`, `reports.resolve`, `moderation.mute`, `moderation.ban`,
  `staff.assign`, `staff.inspect`, `staff.where`, `staff.goto`,
  `staff.transfer`, `staff.restore`, `staff.announce` y `staff.audit`.
- Los roles `Player`, `Moderator`, `Admin` se conservan solo como tier legado,
  moderación y percepción; poseer uno nunca basta para ejecutar un comando ni
  abre por sí solo el canal `staff`. La primera cuenta registrada recibe
  `implementor`; un Admin migrado recibe `administrator`. Cambiar un perfil
  elimina las asignaciones previas y siembra exactamente su bundle; toda acción
  sobre otra cuenta exige rango de perfil estrictamente inferior. Reportes,
  sanciones, asignaciones y acciones staff exigen motivo; la auditoría v11 es
  durable y append-only.
- `staff inspect|where|goto|transfer|restore <nombre> | <motivo>` actúa solo
  sobre objetivos conectados. `staff announce <texto> | <motivo>`, la cola y
  resolución cerrada de reportes, asignación de perfil/capacidad y consulta de
  auditoría comparten Web/Telnet y la misma autorización del actor.
- No existe entrenamiento general por uso. El incremento heredado de `combat`
  tras una derrota de NPC es deuda del mundo de ejemplo.

### Percepción, protocolo y clientes

- Las acciones producen hechos persistentes; percepción proyecta después
  escena, jugador y eventos específicos del observador.
- IDs estables, variantes secretas y contenido oculto quedan en el servidor.
  Los clientes reciben IDs opacos derivados de una sal criptográficamente
  aleatoria nueva por sesión.
- El texto contiene todos los hechos jugables. Imágenes, audio, ANSI, gráficos
  y composición son solo presentación.
- `tiamud.v1` experimental usa secuencias monótonas, deduplicación acotada,
  heartbeats y recuperación autoritativa; no hay replay general de eventos.
- Una recuperación devuelve `sync.scene` y `sync.player` correlacionados, cada
  uno bajo 16 KiB; el navegador valida y aplica el par atómicamente. La ventana
  semántica Web para escenas es 14 KiB y conserva en orden botín propio,
  público y reservado; Telnet usa 48 KiB. Ambos degradan prosa/entidades de
  forma determinista y visible, y los lotes se acotan a 16 eventos antes de
  coalescer y emitir un snapshot autoritativo.
- Todo frame WebSocket normal está limitado a 16 KiB JSON compacto. No evites
  límites de frame, profundidad o cola.
- Una escena GMCP sobredimensionada degrada a un fallback estructurado acotado
  y métrica observable; nunca desaparece en silencio.
- El navegador Svelte 5 implementa acceso registrado, ficha v3, creación,
  habilidades, talentos/promociones, inventario/equipo, mapa, misiones,
  grupos/economía/conversación y preferencias accesibles persistentes.
- Scrollback visible: 500; historial de comandos: 100. Conserva posición de
  lectura y salto a entradas nuevas.
- El Service Worker solo cachea una shell acotada del mismo origen; excluye
  API, WebSocket, imágenes y audio.
- La identidad PWA usa manifiesto same-origin, favicon SVG, iconos PNG 192/512,
  variante maskable 512 y Apple Touch 180 derivados del wordmark existente. La
  puerta web actual pasa 125 Vitest, `svelte-check` 0/0 y build de producción;
  no atribuyas a ello una ejecución Playwright nueva.
- Telnet es cliente textual completo. `tiamud-telnet-codec` es el parser de
  producción compartido con fuzzing. Conserva RFC 1143, TTYPE/MTTS, NAWS,
  CHARSET, GMCP, MSSP, MCCP2 y salida enriquecida opt-in.
- La línea Telnet se acota a 8 KiB y se descarta entera al excederse. Cada GMCP
  se acota a 8 KiB; `Tiamud.Board.Entry` y, defensivamente,
  `Tiamud.Mail.Message` conservan campos no textuales, prefijo UTF-8 del cuerpo
  y `truncated`/`overflow` cuando el JSON escapado no cabe.
- Telnet TLS 1.2/1.3 directo es predeterminado y fail-closed. `plaintext` solo
  es excepción explícita para desarrollo/privado y nunca debe llevar
  credenciales públicas.

### Medios, scripts e IA opcional

- Recursos atmosféricos: sin conexión, inmutables y direccionados por
  contenido. Escenas opacas: AVIF, WebP/JPEG. Sprites: WebP alfa + PNG.
- Manifiestos conservan dimensiones, hashes, licencia, procedencia,
  clasificación, revisión y alt localizado.
- Presentación de entidades: autoría estricta, proyección segura y layout
  determinista. Servidor: 12 anotaciones/32 poses; navegador: 10 sprites más
  overflow semántico.
- Pistas de combate: ataque/golpe/derrota de NPC con IDs opacos. No se infieren
  de prosa. Faltan sprites demostrativos revisados, avatar separado, pista heal
  del servidor y lápida tras derrota.
- Modo lector, Save-Data y multimedia off no descargan sprites; movimiento
  reducido usa pose estática. La capa no es interactiva.
- Lua 5.4 se ejecuta en VM nueva, solo con capacidades, sin sistema de archivos,
  red, procesos, módulos nativos ni debug. Código, memoria, instrucciones,
  tiempo y eventos tienen cuotas; un batch admite ocho módulos y 50 ms.
- IA editorial es opcional, solo de autoría y fuera del juego. `none` es el
  proveedor predeterminado; endpoints locales usan loopback y remotos HTTPS.
  Ningún borrador se publica sin validación y revisión humana.

### Evidencia retenida de inventario 3A–3D

La evidencia histórica de inventario 3A supera formato, pruebas bloqueadas del
workspace y clippy estricto. Las suites enfocadas pasan 77 pruebas de core, 80
de WorldPack, 28 del importador y 195 de `tiamud-server`; la web pasa
`svelte-check` con 0 errores/avisos, 100/100 unitarias, build y 30/30 escenarios
Playwright soportados (15 Chromium y 15 Firefox). Licencias y avisos generados
de 307 dependencias Rust y 76 web también pasan. La suite Node del harness pasa;
humo, carga CI exacta de 16 usuarios
(16 bootstraps/32 rondas, 15 WebSocket + uno Telnet) y soak corto de 30 s con
dos usuarios (64 rondas, readiness 8/8) terminan sin errores y con todos los
veredictos en `pass`. No se ejecutaron PostgreSQL real v10, WebKit local ni soak
de 24 horas. La evidencia operacional procede de un árbol dirty precommit: es
diagnóstico local, no evidencia publicable ni de CI.

La entrega histórica de fase 3B implementó WorldPack v5,
AreaPack v4, snapshot v3, pilas derivadas, contenedores acotados y paridad de
acciones/proyecciones. Aquel árbol integrado pasó formato/check/clippy, 499/499
pruebas Rust (88 core, 93 WorldPack, 28 importador, 202 servidor y 88
restantes), 103/103 web, 30/30 Playwright Chromium/Firefox, 80/80 Node, humo,
carga CI 16 usuarios y soak corto de 66 rondas/10 readiness. También pasaron
`cargo-deny`, licencias y avisos 307 Rust/76 web. No hubo PostgreSQL real v10,
WebKit local ni soak de 24 horas; no conviertas este pase local precommit en
evidencia de CI o producción.

La fase 3C quedó implementada en su árbol integrado mediante snapshot v4, vínculo
durable, tres políticas de transferencia y `give` atómico en Web, Telnet y
GMCP. Lantern Square contiene 9 prototipos y 10 instancias iniciales; su
WorldPack canónico ocupa 49.603 bytes y tiene SHA-256
`f3ab27cb93a2b3c80fe0a9a6ecf8acdd856a45ec1edbda257dd33046a21772c8`.
En aquel árbol pasaron 513/513 pruebas Rust: 93 core, 95 WorldPack, 28 importador,
207 servidor y 90 restantes. También pasaron `svelte-check` con 0 errores y 0
avisos, 103/103 pruebas web, 81/81 Node y 32/32 escenarios Playwright
soportados, 16 Chromium y 16 Firefox. WebKit no arrancó en aquel gate por
bibliotecas GTK/GStreamer ausentes. No se ejecutaron PostgreSQL real v10 ni
soak de 24 horas; esta comprobación local sobre un árbol dirty no es evidencia
de CI, release o producción.

La fase histórica 3D quedó implementada mediante snapshot v5 y
reservas durables de objetos/moneda, revisión y digest canónicos, doble
confirmación, liquidación simultánea y recuperación restart-safe. La evidencia
integrada pasa 550 pruebas Rust: 102 core (80+3+14+5), 97 WorldPack, 28
importador, 231 servidor y 92 restantes —incluidas 23 admin—; también 107
Vitest y 83 Node/bot.
El pase Playwright final completa 36/36 escenarios, 18 Chromium y 18 Firefox.
No convierte
PostgreSQL real v10, WebKit local ni el soak de 24 horas en ejecutados.

### Evidencia integrada de botín 4A

La fase 4A usa WorldPack v6, AreaPack v5, snapshot v6 y esquema SQL v11. El
artefacto canónico de Lantern Square contiene tres salas, nueve prototipos,
nueve instancias iniciales, una tabla y una fuente; ocupa 50.498 bytes y su
SHA-256 es
`3d6c7f40e7a5e56d509b46aeb89439aeea8f5a3168d249dd46609df1fe54c816`.
El árbol integrado pasa 587/587 pruebas Rust (113 core, 240 servidor, 112
WorldPack y 122 restantes), formato y clippy estricto; `svelte-check` 0/0,
109/109 Vitest, 84/84 Node/bot y 19/19 escenarios Chromium. También pasan
`cargo-deny`, licencias y avisos 307 Rust/76 web, humo, la carga exacta de CI
con 16 bootstraps/32 rondas y un soak corto de 30 s con 58 rondas, 127 acciones
y readiness 8/8. No se ejecutaron PostgreSQL real v11, Firefox, WebKit, lector
de pantalla ni soak de 24 horas; la evidencia local precommit no es una
aceptación de release.

### Evidencia integrada de recompensas y afijos 4B

La fase 4B usa WorldPack v7, AreaPack v6, WorldProject v2, snapshot v7 y
esquema SQL v11. El artefacto canónico de Lantern Square contiene tres salas,
diez prototipos, nueve instancias iniciales, dos tablas, una fuente, un afijo y
un pool; ocupa 52.349 bytes y su SHA-256 es
`183e7bc012a9214d68350c1f8fc1f8e6e505bb35c408c14e734901a671b5c7ef`.
El árbol integrado pasa 608/608 pruebas Rust (122 core, 245 servidor, 117
WorldPack y 124 restantes), formato, check, build y clippy estricto; 112/112
Vitest, 86/86 Node/bot y 40/40 escenarios Playwright, 20 Chromium y 20 Firefox.
También pasan `cargo-deny`, licencias/avisos 307 Rust/76 web, humo, carga 16/32
con 15 WebSocket+1 Telnet, cero errores y 55.365.632 bytes de crecimiento RSS
bajo 96 MiB, y soak corto de 30 s con 58 rondas, 127 acciones, readiness 8/8,
cero errores, 45.211.648 bytes totales y 1.536.000 sostenidos. Es evidencia
local precommit dirty: el verificador de aceptación rechaza correctamente el
reporte corto por duración/perfil/warmup/árbol no limpio, no por fallo
funcional. No acredita PostgreSQL real v11, WebKit, lector manual ni soak 24 h.

### Evidencia integrada de telepatía 5A

La fase 5A usa WorldPack v8, AreaPack v7, WorldProject v2, snapshot v7 y
esquema SQL v11. `tiamud:mind-whisper` demuestra coste/cooldown y
`demo:archive` interferencia authored. El artefacto canónico mide 53.040 bytes
y tiene SHA-256
`109670f74056ab67d711fba520f7ad74d1cd16b8c72c06bd257839d89827dbc6`.
El árbol integrado pasa 630/630 pruebas Rust (131 core, 252 servidor, 123
WorldPack y 124 restantes), formato/diff/clippy; check web 0/0, 113/113
Vitest, build, 86/86 Node/bot y 42/42 Playwright soportado (21 Chromium y 21
Firefox). También pasan `cargo-deny` y licencias/avisos 307 Rust/76 web. No se
ejecutaron PostgreSQL real v11, WebKit, lector manual ni soak de 24 h.

### Evidencia integrada de emotes authored 5B

La fase 5B usa WorldPack v9, AreaPack v8, WorldProject v2, snapshot v7 y
esquema SQL v11. `tiamud:smile` demuestra plantillas cerradas y objetivo local
opcional. La recomposición canónica es byte a byte idéntica, mide 53.567 bytes
y tiene SHA-256
`9a75894a9e2e0c48d20257633f323d52bbe78b9db106de8c497f4784392b5e75`.
El árbol integrado pasa 652/652 pruebas Rust (139 core, 257 servidor, 132
WorldPack y 124 restantes), formato/diff/clippy; check web 0/0, 114/114
Vitest, build, 86/86 Node/bot y 44/44 Playwright soportado (22 Chromium y 22
Firefox). También pasan `cargo-deny` y licencias/avisos 307 Rust/76 web. La
auditoría adversarial cerró render recursivo, divergencia validador/runtime y
escape MXP incompleto; quedan 0 P1/P2/P3 abiertos. No se ejecutaron PostgreSQL
real v11, WebKit, lector manual, carga/soak ni soak de 24 h.

### Evidencia integrada de correo offline 5C

La fase 5C mantiene WorldPack v9, AreaPack v8, WorldProject v2 y snapshot v7,
pero avanza SQLite/PostgreSQL al esquema v12. El correo es una superficie
durable separada con buzón, bytes, páginas, caducidad y cuotas por cuenta
acotados; referencias públicas aleatorias; entrega suprimida no reveladora; y
evidencia denunciada de 90 días sin navegación staff de buzones. Web, texto
Telnet, `Tiamud.Mail 1` y el bot estricto comparten el contrato.

El árbol integrado pasa 669/669 pruebas Rust (142 core, 268 servidor, 132
WorldPack y 127 restantes), formato/diff/clippy; check web 0/0, 117/117 Vitest,
build, 87/87 Node y 44/44 Playwright soportado (22 Chromium y 22 Firefox).
También pasan humo, `cargo deny check advisories sources` y licencias/avisos
307 Rust/76 web. La auditoría adversarial deja 0 P1/P2/P3 abiertos. No se
ejecutaron PostgreSQL real v12/mover, WebKit, lector manual, carga/soak ni soak
de 24 h. Véase el
[registro 5C](docs/project/log/2026-07-17-bounded-offline-mail-phase-5c.md).

### Evidencia integrada de tablones comunitarios 5D

La fase 5D mantiene WorldPack v9, AreaPack v8, WorldProject v2 y snapshot v7,
pero avanza SQLite/PostgreSQL al esquema v13. `general` y `newbie` son
tablones globales públicos, acotados, consultables bajo demanda y separados de
chat y correo. La implementación incluye cuota y capacidad transaccionales,
purga fail-closed al arrancar, denuncia única con evidencia temporal,
resolución mediante la cola staff, paridad Web/Telnet/`Tiamud.Board 1` y panel
accesible sin HTML/Markdown remoto. La auditoría cerró pérdida silenciosa del
detalle GMCP, línea Telnet insuficiente, claves JSON duplicadas y mutación Web
optimista: ahora hay fallback UTF-8 explícito, límite de línea 8 KiB, rechazo
recursivo y confirmación correlacionada que preserva borradores.

El árbol final pasa formato/diff/clippy estricto y 691/691 pruebas Rust: 146
core, 283 servidor, 132 WorldPack y 130 restantes. PostgreSQL 18.4 real pasa
19/19 en adaptador+mover v13, incluida migración de una base actual
representativa; las 283 de servidor incluyen flujo board/runtime y lock
advisory concurrente reales, además de 5/5 focused PostgreSQL. Web pasa check
0/0, 122 Vitest, build y 46/46 Playwright soportado (23 Chromium y 23 Firefox);
Node pasa 88, además de humo, advisories/sources y licencias/avisos 307 Rust/76
web. La auditoría deja 0 P1/P2/P3 abiertos. No se ejecutaron WebKit por faltar
bibliotecas del host, lector manual, carga/soak mixtos ni soak de 24 h. TLS,
backup/restore, monitorización y recuperación PostgreSQL de producción siguen
siendo responsabilidad de cada instalación. Véase el
[registro 5D](docs/project/log/2026-07-17-bounded-moderable-community-boards-phase-5d.md).

### Voz espacial 5E implementada

[ADR 0019](docs/decisions/0019-bounded-spatial-voice.md) está implementado sin
cambiar WorldPack v9, AreaPack v8, WorldProject v2, snapshot v7 ni esquema SQL
v13. Core, actor, Web, Telnet/GMCP, cliente y bot demuestran radio 1/3, BFS
dirigido, ciclos/convergencia, exclusión de salidas secretas, máximo de 259
salas, mute/bucket, ignore/block/deaf y rollback con cero entregas. La evidencia
integrada se conserva en el
[registro 5E](docs/project/log/2026-07-17-bounded-spatial-voice-phase-5e.md).

### Evidencia integrada de catálogo y bundles locales 6A

La fase 6A pasa 144/144 pruebas de `world-pack`, incluidas 10/10 del CLI, y
707/707 pruebas Rust del workspace. También pasan formato/diff, clippy estricto
del workspace, check MSRV con Rust 1.88, el E2E de tres bundles cuyo catálogo
es `e954be5b7e1d6918a1b87d5405d651f0fc52ee81996ab0c100cfcb1909ed15a6`,
humo, `cargo deny check advisories sources` y licencias/avisos 308 Rust/76 web.
No cambia Web, protocolo, runtime, snapshot ni SQL y no aporta una nueva puerta
Web/PostgreSQL. Véase el
[registro 6A](docs/project/log/2026-07-17-area-catalog-bundles-phase-6a.md).

### Carga local aditiva de áreas 6B implementada

[ADR 0020](docs/decisions/0020-additive-local-area-loading.md) y su código
implementan generación fijada, bootstrap inicial/completo, `DeferredExit`,
coordinación 2 activas + 8 pendientes/5 s/32 áreas, staging externo, instalación
y rollback conjuntos, persistencia condicional, error público genérico y
métricas de baja cardinalidad. Las suites enfocadas cubren core, `world-pack`,
coordinador, rollback durable del actor y rutas SQLite de recovery. Un humo real
con el catálogo
`e954be5b7e1d6918a1b87d5405d651f0fc52ee81996ab0c100cfcb1909ed15a6`
abre Web/Telnet y readiness, deja dos áreas residentes en arranque vacío y
cierra limpio. La evidencia agregada exacta se conserva en el
[registro 6B](docs/project/log/2026-07-17-additive-local-area-loading-phase-6b.md).

### Sustitución quiescente de generaciones 6C implementada

[ADR 0021](docs/decisions/0021-quiescent-area-generation-cutover.md) y el árbol
integrado implementan staging completo, compatibilidad estricta, barrera de
admisión, corte a cero sesiones/leases, puntero durable SQL v14, CAS conjunto
con snapshot v7, recovery exacto y rollback explícito. La puerta final pasa 774
pruebas Rust, 329 del servidor, formato/diff, clippy, MSRV, cargo-deny,
licencias/avisos 308 Rust/76 web, Svelte 0/0 y 125 Vitest; la reauditoría queda
en 0 P1/0 P2. PostgreSQL 18 desechable pasa 20/20 pruebas `--all-targets` de
`tiamud-persistence-postgres` y 7/7 pruebas enfocadas
`persistence_postgres::tests::postgres_` del servidor, incluida la CAS v14. El
humo PostgreSQL abre esquema 14, liga G0 `e954be…15a6` en secuencia 1, confirma
readiness PostgreSQL/mundo, Web 200, banner Telnet, dos áreas residentes,
SIGHUP `noop=1` y apagado actor/store limpio. Separadamente, el humo 6C sobre
SQLite activa G1, revierte a G0 y recupera por restart con métricas exactas. La
evidencia exacta está en el
[registro 6C](docs/project/log/2026-07-17-quiescent-area-generation-cutover-phase-6c.md)
y el [registro PostgreSQL/PWA](docs/project/log/2026-07-17-postgresql-v14-and-pwa-acceptance.md).

### Retirada reversible de unidades de área 6D parcial

[ADR 0022](docs/decisions/0022-reversible-area-unit-retirement.md) fija el
contrato y el árbol actual implementa sus formatos v8/v15, checkpoints de NPC y
objetos regionales, pins cerrados, SCC/cierre inverso, desmontaje de contenido y
CAS de retiro/rehidratación en ambos adaptadores. Existe una ruta interna del
actor para retirar una unidad vacía de la misma revisión y rehidratarla ante
entrada; recovery falla cerrado si manifiesto y checkpoints no coinciden. La
fase permanece parcial hasta completar el E2E de restart, una superficie
operativa acotada y la medición de ahorro real. La puerta local pasa 792/792
pruebas Rust; además PostgreSQL v15 pasa 22/22 del adaptador+mover + 8/8 del servidor.
No existe LRU ni política automática de memoria.

### Evidencia histórica retenida

La memoria fechada conserva este baseline de 2026-07-16. Vincula aquel árbol,
no autoriza saltarse pruebas tras cambios:

- 409 pruebas Rust bloqueadas y clippy estricto; 158 pruebas enfocadas de
  `tiamud-server`.
- 96 pruebas web y 28 escenarios combinados Chromium/Firefox.
- PostgreSQL 18.4 validó **esquema v8**, restauración SQLite, mover, juego mixto
  y reinicio autenticado. No se relabela como aceptación v10.
- Perfil mixto de 30 s en `2b18cfe`: 66 rondas, 145 acciones, 10/10 readiness,
  cero errores/drops y cero buzones vivos al apagar.
- Fuzz del parser Telnet: 3.081.428 ejecuciones en 121 s sin crash, timeout, OOM
  ni artefacto de fallo.
- Una captura Chromium autenticada real detectó y cerró defectos de auth y
  cancelación del manifiesto. El hero es decorativo; la captura de juego es
  evidencia de producto.

GitHub Actions puede fallar antes de ejecutar por facturación/límite de gasto.
Inspecciona el job real; no conviertas una insignia en evidencia.

## Restricciones de diseño no negociables

1. Un host y un servidor modular siguen siendo el despliegue de referencia. No
   añadas Kubernetes, Redis, object storage obligatorio, microservicios o una
   segunda autoridad como atajo.
2. Conserva un escritor y un sistema de persistencia seleccionado. Nunca doble
   escritura ni consistencia checkpoint/rollback debilitada.
3. Dominio neutral al transporte. Una acción web no evita reglas de Telnet.
4. Autorización y percepción antes de serializar. Presentación nunca revela ni
   crea visibilidad.
5. Texto plano completo y accesible. Perder medios/color no pierde acciones.
6. Acota todo input/output no confiable: bytes, profundidad, cantidades, colas,
   concurrencia, descompresión, scripts, recursos, reintentos y tiempo.
7. Prefiere datos deterministas y transformaciones puras. Sin IA runtime,
   guiones arbitrarios de progresión, medios remotos ni estado derivado de
   prosa.
8. IDs publicados son identidad durable. Tras release v3, renombrar exige
   alias/tombstone/migración explícita.
9. Autoridad de staff separada de XP, nivel, clase y talentos. Usa la capacidad
   concreta y auditoría append-only; perfiles y títulos son asignación/presentación,
   no autorización implícita.
10. Las fases 3A–4B ya separan prototipo/instancia, derivan pilas, aplican
    capacidad recursiva, ejecutan `give` y trade, y resuelven botín repetible
    con procedencia, derechos round-robin, pass, recompensas selladas y afijos
    en commits atómicos. Conserva esos invariantes antes de añadir otro reparto.
    Cajas, llaves y tiradas pagadas quedan fuera.
11. Optimiza con evidencia. Siguiente ruta: dirty tracking y deltas/coalescing,
    conservando snapshot atómico.
12. La carga local aditiva 6B y la retirada interna parcial 6D existen, pero no
    las presentes como sustitución de revisión, LRU caliente o política de
    memoria aceptada. Tampoco afirmes producción pública, WebKit actual,
    prueba manual de lector, soak 24 h, release pública/etiquetada ni procedencia
    OIDC verificada.

## Mapa del repositorio

| Ruta | Responsabilidad |
| --- | --- |
| `crates/mud-core/` | Dominio, acciones, progresión y percepción |
| `crates/mud-server/` | Actor, auth, transportes, routing, carga y fachada de persistencia |
| `crates/persistence-sqlite/` | SQLite esquema v15 y almacenamiento local |
| `crates/persistence-postgres/` | PostgreSQL esquema v15, adaptador y mover |
| `crates/world-pack/` | WorldPack/AreaPack, validación, composición y CLI |
| `crates/scripting/` | Capacidades Lua con cuotas |
| `crates/asset-pipeline/` | Compilador de medios determinista |
| `crates/telnet-codec/` | Parser Telnet de producción compartido con fuzzing |
| `crates/cell-frame/` | Renderizado de celdas acotado |
| `crates/editorial-ai/` | Adaptadores opcionales de autoría |
| `crates/tiamud-admin/` | Integridad, migración y recuperación SQLite |
| `apps/web/` | Cliente Svelte y cobertura Playwright |
| `clients/` | Integraciones TinTin++ y Mudlet |
| `tools/envy-import/` | Importador histórico conservador |
| `worlds/` | Paquete canónico y áreas fuente |
| `scripts/` | Humo, carga, soak, release y licencias |
| `fuzz/` | Corpus, diccionarios y targets acotados |
| `docs/` | Especificaciones, ADR, operación, memoria y evidencia |
| `deploy/` | Caddy, systemd y entorno revisados |

## Flujo de cambios

### Antes de editar

- Inspecciona `git status` y conserva todo trabajo existente.
- Lee módulo, pruebas, ADR/diseño, memoria y fila de matriz relevantes.
- Identifica primero la frontera autoritativa: dominio, persistencia,
  percepción, protocolo, presentación o autoría.
- Prefiere cambios enfocados y reversibles; no tapes defectos de dominio solo
  en un cliente.

### Durante la edición

- El comportamiento vive en `tiamud-core`; transportes equivalentes lo adaptan.
- Persistencia suele exigir paridad SQLite/PostgreSQL, migración/restore y
  revisión de snapshot.
- WorldPack suele exigir esquema, validador, normalización, composición,
  loader, datos revisados y CLI tests.
- Protocolo web suele exigir serialización Rust, parser TypeScript estricto,
  socket/recovery, Playwright y revisión Telnet/GMCP.
- Medios pasan por manifiestos revisados y percepción.
- Usa aritmética checked/saturating y límites explícitos; falla cerrado ante
  autoridad, esquema o seguridad ambiguos.
- Un cambio material actualiza matriz, memoria y registro fechado.

### Archivos generados y sensibles

- No edites `THIRD_PARTY_NOTICES.md` a mano; regenera/verifica con el script.
- Conserva licencias completas y hashes de verificación.
- Construye artefactos mundo/medios con herramientas del repo.
- Nunca comitees `.env`, bases, logs operativos, reportes locales crudos,
  credenciales, tokens, claves o certificados.

## Verificación

Usa la puerta mínima al iterar y amplía según riesgo. No afirmes un pase de un
comando no ejecutado sobre el árbol final.

### Rust

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

PostgreSQL requiere una base desechable y autoridad explícita:

```sh
TIAMUD_TEST_POSTGRES_URL='postgresql://.../postgres?sslmode=disable' \
  cargo test --locked -p tiamud-persistence-postgres --all-targets
```

Nunca apuntes pruebas destructivas a producción.

### Navegador

```sh
pnpm --dir apps/web install --frozen-lockfile
pnpm --dir apps/web check
pnpm --dir apps/web test
pnpm --dir apps/web build
pnpm --dir apps/web e2e
```

Reproduce CI con Node 24/pnpm 11.11. El paquete acepta pnpm 10+. En el host
actual Chromium/Firefox funcionan; WebKit carece de bibliotecas GTK/GStreamer.

### Proyecto, transporte y cadena de suministro

```sh
bash scripts/verify-project-licenses.sh
node scripts/generate-third-party-notices.mjs --check
node --test scripts/bot.test.mjs scripts/verify-soak-report.test.mjs scripts/telnet-bot.test.mjs
node scripts/smoke.mjs
node scripts/load.mjs --users 2 --telnet-users 1 --rounds 2 \
  --concurrency 2 --pace-ms 100
```

El harness carga/soak necesita Node 24. Un perfil corto es diagnóstico, no
prueba de 24 horas.

### Autoría

```sh
mkdir -p build/worlds
cargo run --locked -p world-pack --bin tiamud-world -- compose \
  worlds/lantern-square.project.json \
  --output build/worlds/lantern-square.composed.world.json
cargo run --locked -p world-pack --bin tiamud-world -- validate \
  worlds/lantern-square.world.json
cargo test --locked -p world-pack
```

### Fuzzing

```sh
cargo +nightly-2026-07-01 fuzz run telnet_parser -- \
  -dict=fuzz/dictionaries/telnet_parser.dict \
  -max_len=262144 -timeout=5 -rss_limit_mb=1024 -max_total_time=120
```

## Límites actuales honestos

- sin soak de 24 horas retenido;
- sin WebKit local actual ni auditoría manual con lector real;
- sin CI alojado actual: GitHub Actions está bloqueado antes de ejecutar pasos
  por facturación o límite de gasto;
- sin sprites de demostración revisados, aunque el puente técnico existe;
- carga local aditiva 6B y sustitución quiescente 6C implementadas; 6D dispone
  de contratos y vertical interna de retirada/rehidratación de la misma revisión
  pero sigue parcial, sin sustitución individual, jugadores durante retirada,
  migraciones, política/LRU, GC, red, compresión o firma remota;
- sin despliegue/repo público, hito estable, release etiquetada o procedencia
  verificada;
- sin entrenamiento general, migración general de IDs, multiclase, sprite de
  jugador, heal cue del servidor ni lápida de derrota;
- fases social 1, telepatía 5A, emotes authored 5B, correo offline 5C,
  tablones comunitarios 5D y voz espacial 5E implementadas; sin historial
  general, telepatía de grupo/offline ni
  emotes globales/de grupo;
- fase staff 2 implementada; sin OLC, spawn/purge, acceso al host ni autoridad
  obtenible por rol legado, título, XP o WorldPack;
- fase 4B de inventario/botín implementada: prototipos/instancias, creación durable,
  pilas derivadas, contenedores con capacidad/profundidad acotadas, cantidades,
  paginación, rareza, `itemLevel`, vínculo, políticas, `give`, trade confirmado,
  botín repetible, procedencia, round-robin, pass, recompensas selladas ganadas
  y afijos deterministas; sin need/greed, contribución ni master-loot;
- backup/TLS/monitorización/recuperación PostgreSQL dependen del operador;
- marca/dominios y derechos de terceros siguen siendo trabajo legal externo.

Un workflow configurado no demuestra que se ejecutó. Inspecciona sus pasos.

## Criterio de finalización

Una tarea termina cuando el comportamiento existe en todas las fronteras
afectadas, hay pruebas enfocadas y puertas amplias proporcionales al riesgo, la
documentación es factual, los artefactos generados están sincronizados y
`git diff` contiene solo cambios intencionados. Expón límites; no conviertas una
arquitectura prevista o CI configurada en una afirmación de implementación.
