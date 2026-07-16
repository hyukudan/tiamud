# Especificación técnica de Tiamud v0.3

Estado: especificación de implementación en prelanzamiento, 2026-07-16.

Este documento registra las decisiones actuales de producto y arquitectura de
Tiamud. Sustituye la planificación de personajes y multimedia de la
[especificación histórica v0.2](spec-v0.2.md). Los ADR aceptados y el código
implementado prevalecen cuando este resumen sea menos específico.

## Contrato de producto

Tiamud es un mundo textual multijugador persistente con presentación ilustrada
opcional. Un estado de mundo autoritativo sirve al navegador, Telnet y clientes
MUD compatibles.

El producto debe seguir siendo:

- completamente jugable como texto plano;
- orientado al navegador sin convertirlo en autoridad;
- sencillo de ejecutar en un equipo corriente;
- determinista y acotado en toda frontera no confiable;
- definido por datos para mundo, personajes y medios;
- accesible sin imágenes, color, audio ni movimiento;
- compatible con importación Envy/Merc revisada, sin ejecutar programas
  históricos ni presuponer derechos; e
- independiente de IA, GPU, Kubernetes, Redis o cloud obligatorios.

La IA puede ayudar al desarrollo o autoría sin conexión. Es opcional, acotada,
revisada y nunca participa en el juego autoritativo.

## Despliegue y persistencia

Un servidor Rust posee un actor de mundo y expone HTTP/WebSocket y Telnet. Los
recursos web pueden servirse desde el proceso o un proxy en el mismo host.
PostgreSQL se recomienda para instalaciones públicas nuevas; SQLite es la
alternativa local sin servicio. Se selecciona uno y nunca se hace doble
escritura.

Ambos adaptadores implementan el esquema v10. Conserva las suscripciones
durables v9 y añade perfiles/capacidades de staff, resolución cerrada de
reportes y auditoría append-only. El traslado SQLite→PostgreSQL copia estas
relaciones en una transacción. La aceptación PostgreSQL real retenida es v8;
no se relabela como aceptación v10 y aún falta ejecutar v10 contra un servidor
PostgreSQL real.

La operación está en [single-host.md](operations/single-host.md). Microservicios
y orquestación no son objetivo sin carga medida que lo justifique.

## Datos de mundo y autoría

WorldPack v4 es el objetivo canónico. AreaPack v3 permite composición modular
estricta. WorldPack v3/v2, AreaPack v2/v1 y Envy/Merc se normalizan al importar
o componer; el dominio no conserva vocabulario heredado.

Propiedades obligatorias:

- IDs estables, namespaced, en minúscula;
- estructuras acotadas y validación de referencias;
- artefactos y digests canónicos deterministas;
- pruebas de contenido y vistas previas revisadas;
- sin conversión de imagen online ni IA generativa en juego;
- origen, licencia, procedencia y revisión de medios; y
- resolución en servidor de secretos y variantes de percepción.

Consulta [world-authoring.md](world-authoring.md) y
[character-progression-v3.md](design/character-progression-v3.md).

### Objetos e inventario 3A

WorldPack v4 separa `itemPrototypes`, que contienen contenido y reglas
reutilizables, de `initialItemInstances`, que asignan cada unidad durable a una
sala, un `npc_spawn` o `staged`. En runtime `ItemInstanceId` es una cadena
tipada interna; las instancias nuevas usan una secuencia monotónica incluida en
checkpoint, rollback y snapshot. Los clientes nunca reciben esa identidad ni
`prototypeId`: Web, Telnet y GMCP operan con IDs opacos derivados de una sal
criptográficamente aleatoria nueva por sesión. Telnet muestra la referencia de
cada unidad visible o portada y permite emplearla como selector exacto.

`entityMedia(kind=item)` referencia un prototipo reusable; el servidor aplica
esa presentación solo a sus instancias ya visibles. En cambio,
`roomPerception.hiddenItems` referencia la instancia concreta y no cambia la
visibilidad de las demás unidades del mismo prototipo.

Cada prototipo declara peso, coste de slot, rareza cerrada
`common|uncommon|rare|epic|legendary`, `itemLevel`, política de pila y política
de transferencia. La fase 3A acepta solo `unique` y `transferable`; los valores
`character_bound` y `non_transferable` están definidos pero se rechazan como
reservados hasta que 3C ejecute sus reglas. No hay pilas. Cada personaje tiene
límites de slots y peso. El peso incluye inventario
y equipo; los slots cuentan las unidades portadas, por lo que equipar libera
slots y desequipar debe reservarlos. Take, unequip, reemplazo de equipo y
concesiones de script prevalidan la operación completa y fallan sin mutación si
excede capacidad.

Rareza, nivel y peso aparecen como texto en escena, inventario y equipo. El
color nunca es la única señal. La capacidad se calcula exclusivamente en el
servidor y se proyecta como uso/máximo.

`WorldRuntimeSnapshot` v2 conserva `itemInstances` y el siguiente contador. La
lectura de v1 pasa por un DTO y una conversión explícitos; referencias
huérfanas, ubicaciones incompatibles o contadores inválidos bloquean el
arranque. La restauración conserva instancias iniciales nuevas del WorldPack
actual y superpone el estado durable, sin resucitar instancias runtime ausentes
del snapshot. Antes de abrir listeners se leen por lotes, se validan y se
descartan los agregados de propietarios conectados o desconectados: existencia,
equipo y capacidad deben concordar con las ubicaciones de instancia. En 3A ese
snapshot acotado sigue siendo la única autoridad durable:
SQLite y PostgreSQL lo confirman en su transacción de esquema v10 junto con los
demás cambios. No hay tabla espejo ni esquema v11.

Los límites principales son 1.024 prototipos y 8.192 instancias iniciales por
WorldPack, 65.536 instancias runtime, 1 MiB para el snapshot y 1 MiB agregado
para las fichas de propietarios inspeccionadas al arrancar; cada ficha tiene un
máximo de 64 KiB.

## Modelo de personaje

```text
baseline neutral de ficha
  + concesiones de creación del linaje
  + concesiones del trasfondo opcional
  + concesiones de creación de clase base
  + concesiones de nivel de clase activa
  + rangos adquiridos de talentos de linaje/clase
  + concesiones únicas de promoción avanzada
  = ficha permanente

equipo + estados activos se aplican después
```

El baseline neutral no aporta atributos, skills o capacidad oculta. La clase
base define el inicio real, con adiciones de linaje/trasfondo. La creación llena
vida y recursos una vez; aumentar capacidad después conserva valores actuales.

Términos canónicos: linaje, trasfondo, clase base, talento/especialización y
clase avanzada. `Race` y `archetype` solo son sinónimos de importación/UI.

La primera versión usa:

- una curva acumulada de XP/nivel global;
- una clase activa;
- reservas acotadas separadas para clase y linaje;
- adquisición de talento distinta de promoción;
- requisitos declarativos y concesiones verificadas;
- linaje/trasfondo inmutables tras crear; y
- sin multiclase, XP de clase, promoción reversible ni scripts arbitrarios de
  concesiones permanentes.

Evolución de linaje desarrolla nodos acotados, no sustituye el linaje. La
promoción avanzada cambia identidad atómicamente conservando XP/nivel.

| Dimensión | Definiciones de demostración |
| --- | --- |
| Linajes | Mortal, Stonekin, Embermarked |
| Trasfondos | Caravan Scout, Archive Apprentice, City Guard |
| Clases base | Wanderer, Vanguard, Arcanist |
| Clases avanzadas | Pathfinder, Bastion, Stormcaller, Spellblade |

La curva de ejemplo define `100 × (level - 1)²` para niveles 2–10. Pathfinder,
Bastion y Stormcaller parten de su base al nivel 5; Spellblade permite entrada
Vanguard o Arcanist al nivel 6 con requisitos alcanzables.

Gramática, grafos e invariantes son normativos en el
[diseño v3](design/character-progression-v3.md) y
[ADR 0007](decisions/0007-worldpack-v3-character-model.md).

## Combate, habilidades y seguridad de progresión

Acciones marciales, hechizos y utilidad comparten un modelo acotado con costes,
cooldowns, objetivos, daño/curación, resistencias y estados temporales. Los NPC
usan definiciones iguales y decisión determinista por reglas, nunca IA
generativa runtime.

`martial`, `spell`, `utility` son categorías comunes a WorldPack, Rust y wire
del navegador. `innate` pre-v3 no forma parte del modelo limpio.

Autoría y runtime validan límites acumulados. Aumentar vida máxima no cura;
aumentar capacidad de recurso no rellena y una reserva nueva empieza vacía.
Las acciones permanentes se aplican atómicamente y una sola vez.

La recompensa hard-coded de skill de combate y la misión de demostración son
deuda de política del ejemplo. Deben pasar a señales/recompensas de autoría
antes de un contrato de contenido estable.

## Sistemas sociales, grupos, staff y botín

La fase social 1 está implementada. `say` es local; `tell` llega a un personaje
conectado; `reply` usa en memoria el ID del último remitente cuyo `tell` se
entregó. Los grupos de hasta seis miembros persisten
create/invite/accept/revoke/kick/transfer/disband/leave/status y ofrecen
`party say`, `gsay`, `gtell`.

`general`, `newbie`, `staff` exigen suscripción durable. Unirse, enviar y
recibir `staff` requiere perfil `moderator`, `administrator` o `implementor`;
`helper` y `builder` no acceden. La membresía del grupo sobrevive a la
desconexión, pero accept/revoke/kick/transfer resuelven un nombre conectado y no
administran un objetivo offline.

La misma política, rate limit, mute, ignore/block y buzones por sesión sirve a
acciones web, comandos Telnet y `Comm.Channel.Text` GMCP. No existen correo o
historial durable, shout/yell espacial, telepatía authored, follow/assist/
rescue, selección general de aliados, XP/misión compartida ni reparto de botín.

La autoridad ejecutable de staff es independiente de la ficha y usa una de
doce capacidades cerradas: `reports.view`, `reports.resolve`,
`moderation.mute`, `moderation.ban`, `staff.assign`, `staff.inspect`,
`staff.where`, `staff.goto`, `staff.transfer`, `staff.restore`,
`staff.announce`, `staff.audit`. Nunca se obtiene por XP, nivel, clase,
talentos o WorldPack.

Los perfiles operativos cerrados son `helper`, `builder`, `moderator`,
`administrator`, `implementor`. Siembran bundles revisados de capacidades y
pueden llevar un `lore_title` opcional; perfil y título no sustituyen la
comprobación de capacidad. Los roles `Player`, `Moderator`, `Admin` se
conservan solo como tier legado para moderación y percepción: por sí solos
nunca autorizan un comando ni abren el canal `staff`. Ese canal requiere perfil
`moderator`, `administrator` o `implementor`; `helper` y `builder` no acceden.
La primera cuenta registrada recibe `implementor`; las cuentas Admin migradas
reciben `administrator`. Cambiar perfil elimina las asignaciones anteriores y
siembra exactamente el bundle del perfil nuevo. La jerarquía impide actuar
sobre la propia cuenta o un rango igual/superior.

La fase 2 implementa cola de reportes y resolución `dismissed`,
`action_taken` o `no_action`; `inspect`, `where`, `goto`, `transfer`,
`restore`, `announce`, consulta de auditoría y asignación explícita de
perfil/capacidad. Todas exigen motivo. Las acciones que tocan el mundo resuelven
un personaje conectado; asignaciones y reportes usan identidad durable. Las
mutaciones sensibles y su auditoría se confirman atómicamente, y `staff_audit`
rechaza actualización y borrado.

Sintaxis textual normativa de fase 2:

```text
staff
staff status
staff reports <motivo>
staff resolve <id> <dismissed|action_taken|no_action> <motivo>
staff inspect <nombre> | <motivo>
staff where <nombre> | <motivo>
staff goto <nombre> | <motivo>
staff transfer <nombre> | <motivo>
staff restore <nombre> | <motivo>
staff announce <texto> | <motivo>
staff audit <motivo>
staff profile <nombre> <helper|builder|moderator|administrator|implementor|none> <título|none> | <motivo>
staff capability <grant|revoke> <nombre> <capacidad> | <motivo>
```

Web expone acciones estructuradas estrictas equivalentes. Telnet usa el mismo
analizador; los anuncios llegan como `announcement` en texto, Web
`chat.message` y GMCP `Comm.Channel.Text`.

Los objetos separan prototipo reutilizable e instancia durable con ubicación
autoritaria. Los NPC poseen instancias de botín fijo validadas y las
transfieren al morir. Capacidad por slots/peso, rareza textual y `itemLevel`
están implementados. Faltan pilas, contenedores, give/trade, afijos, propiedad y
reparto de grupo, procedencia de drops repetibles y tablas ponderadas. Cajas de
botín pagadas o tiradas con moneda prémium quedan fuera del baseline.

Consulta la [hoja de ruta social/staff/botín](design/social-staff-and-loot-roadmap.md).

## Escenas semánticas y medios

Una escena es una proyección semántica segura ante percepción. Prosa y
entidades estructuradas contienen todo. La imagen atmosférica es opcional.

El compositor añade solo presentación a entidades visibles:

- poses transparentes revisadas de objetos/NPC;
- huella, prioridad y capa normalizadas;
- slots deterministas y zonas seguras en cliente;
- máximo de diez sprites y overflow semántico;
- pistas cortas `attack`/`hit`/`defeat` derivadas por servidor (`heal` queda
  reservado en cliente); y
- gating completo para lector, Save-Data, movimiento, colores y multimedia.

Las anotaciones viajan como sidecar web, no en `SceneView`, evitando expansión
GMCP y fugas. El servidor decide visibilidad/sprite; manifiestos, geometría; el
navegador, colocación adaptable. Layout es función pura del snapshot.

El juego nunca espera medios. Las pistas decaen a `idle` y no se reproducen tras
resync. Un snapshot de la misma vista retiene la pista solo si el NPC opaco
sigue presentado; retirada/cambio/resync limpia. No hay jugador compuesto ni
lápida persistente en v1. Consulta
[scene-compositor-v1.md](design/scene-compositor-v1.md) y
[ADR 0008](decisions/0008-optional-scene-compositor.md).

## Formatos multimedia

- Atmósfera opaca: AVIF, luego WebP/JPEG.
- Sprite transparente: WebP alfa + PNG.
- AVIF alfa: aplazado hasta demostrar ruta RGBA determinista útil.
- Atmósfera transparente/lossless: PNG.
- SVG/HTML/CSS/JavaScript, hosts remotos arbitrarios y `data:` inline:
  rechazados.

Recursos mismo-origen, inmutables y direccionados por contenido. Poses estáticas
en v1; animación CSS acotada. Vídeo, WebP animado y composición runtime quedan
aplazados.

## Protocolo

`tiamud.v1` WebSocket sigue experimental. Clientes reciben IDs opacos derivados
de una sal criptográficamente aleatoria nueva por sesión, además de revisiones
y secuencias por sesión. Texto y percepción preceden cada representación.

Salvaguardas implementadas:

- 16 KiB compactos por frame WebSocket de salida;
- recuperación correlacionada `sync.scene`/`sync.player`, aplicada como par; y
- escena GMCP grande degradada a escena mínima más aviso acotado, nunca silencio.

Antes del primer hito público, cambios densos deberían migrar a batches
atómicos `baseRevision`/`revision`. Las pistas decoran `message.append`; no crean
secuencia ni se infieren de prosa. Referencia:
[tiamud-v1.md](protocol/tiamud-v1.md).

## Invariantes de accesibilidad y seguridad

- Una imagen nunca contiene información exclusiva.
- Modo lector desactiva arte decorativo salvo petición explícita.
- Texto de jugador no se convierte en ANSI/MXP/HTML/CSS/URL/alt sin escape y
  validación contextual.
- Gráficos terminales requieren capacidad y consentimiento explícitos.
- Recursos pesados viajan por HTTPS, no GMCP/Telnet.
- Parsers, subnegociaciones, profundidad JSON, colas, scripts, recursos y
  descompresión tienen límites.
- Entidades ocultas, salas secretas y variantes no resueltas no se precargan.

## Estado de implementación

| Área | Estado al actualizar v0.3 |
| --- | --- |
| Combate y habilidades acotadas | Baseline implementado y probado |
| Límites acumulados y nivel sin relleno | Implementado y probado |
| WorldPack v4/AreaPack v3, validador, normalizador y catálogo | Integrado en servidor; entradas WorldPack v3/v2 y AreaPack v2/v1 se normalizan |
| Linaje/clase/talento/promoción y persistencia | Implementado y probado |
| Creación v3 navegador/Telnet | Default/none/explícito implementado |
| Ficha, talentos y promociones web | Implementado y probado |
| Recuperación dividida, frame gate y fallback GMCP | Implementado y probado |
| Compositor navegador y accesibilidad | Implementado y probado |
| Perfil de sprite transparente | WebP alfa + PNG implementado |
| Sidecar de entidades seguro | Implementado; sin sprites de muestra |
| Pistas de combate | Ataque/golpe/derrota NPC; sin jugador/heal/lápida |
| Social fase 1 y esquema v10 | Grupos/chat/reply/canales y paridad Web/Telnet/GMCP implementados; aceptación PostgreSQL real v10 aún no retenida |
| Staff fase 2 | Perfiles/capacidades, reportes, acciones acotadas, razones, Web/Telnet y auditoría append-only implementados; aceptación PostgreSQL real v10 aún no retenida |
| Inventario fase 3A | Prototype/instance, snapshot v2 migrado desde v1, slots/peso, rareza/`itemLevel`, IDs opacos y paridad Web/Telnet/GMCP implementados; puertas locales integradas superadas |
| Protocolo atómico de delta denso | Pendiente |

En el árbol integrado de fase 3A pasan formato, pruebas bloqueadas del workspace
y clippy estricto; 77 pruebas de core, 80 de WorldPack, 28 del importador y 195
del servidor; `svelte-check` sin errores ni avisos, 100/100 pruebas web y build;
30/30 escenarios Playwright soportados; y las comprobaciones de licencias y
avisos generados para 307 dependencias Rust y 76 web. No se ejecutaron
PostgreSQL real v10, WebKit local ni soak de 24 horas. Estas puertas locales no
demuestran por sí solas publicación o CI.

Una pieza aislada no es una funcionalidad end-to-end. Actualiza documentación
cuando las puertas de integración cambien.

## Puertas de aceptación

Personajes v3:

1. Selección de linaje/trasfondo/clase en navegador y texto.
2. Ficha acotada idéntica en cada persistencia.
3. XP concede puntos sin curar/recargar.
4. Talentos/exclusiones atómicos y durables.
5. Cada clase avanzada tiene ruta alcanzable.
6. Spellblade cubierto desde Vanguard y Arcanist.
7. Grafos inválidos/cíclicos/sobre límite no publican.
8. V2/Envy normaliza determinista o falla con diagnóstico.

Compositor:

1. Assets solo para entidades visibles.
2. Assets inválidos no cambian listas/acciones.
3. Layout determinista, adaptable, acotado y overflow textual.
4. Lector/Save-Data no descarga sprites.
5. Movimiento reducido muestra resultado estático.
6. Pistas derivadas de eventos y eliminadas en resync.
7. Sin exceder presupuesto WebSocket/GMCP.
8. Telnet plano completo y limpio.

## Trabajo aplazado o rechazado

Aplazado hasta necesidad demostrada: transformaciones, culturas, respec,
rangos de clase, editor de grafos, simulación de balance, medios animados,
gráficos raster terminales y carga caliente. El compositor necesita sprites
revisados; avatar, heal cue y lápida son mejoras opcionales.

Rechazado como baseline: Kubernetes, microservicios, Redis, object storage
obligatorio, generación de imagen runtime, decisiones LLM runtime, layout de
píxeles en servidor, física espacial continua, Canvas/WebGL para diez sprites,
scripts generales de progresión y blobs multimedia por Telnet.
