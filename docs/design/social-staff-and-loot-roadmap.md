# Hoja de ruta de sistemas sociales, staff e inventario/botín

Estado: auditoría de implementación y diseño de continuación, 2026-07-17.

Este documento distingue lo que Tiamud implementa hoy de los sistemas que son
habituales en algunos MUD clásicos o convenientes para un mundo persistente
moderno. No convierte cada antecedente histórico en un requisito. El código y
las pruebas siguen siendo la autoridad sobre comportamiento actual; la
[matriz de completitud](../project/COMPLETION_MATRIX.md) conserva la autoridad
sobre la aceptación de la especificación histórica.

## Resumen ejecutivo

- Sí existen grupos persistentes de hasta seis miembros: crear, invitar,
  aceptar, revocar, expulsar, transferir liderazgo, disolver, salir y consultar
  estado, además de `party say` con alias `gsay`/`gtell`.
- Sí existen `reply` en línea y los canales suscritos `general`, `newbie` y
  `staff`; este último exige perfil `moderator`, `administrator` o
  `implementor`. No existen correo sin conexión, historial durable de
  conversación ni una habilidad de telepatía.
- Sí existen los perfiles `helper`, `builder`, `moderator`, `administrator` e
  `implementor`, doce capacidades cerradas y un título narrativo opcional. La
  XP, la ficha, WorldPack, el perfil visible y el rol legado nunca sustituyen
  la capacidad concreta.
- Sí existen comandos de moderación dentro del juego: reportar, silenciar,
  expulsar, revertir sanciones y cambiar roles según autoridad.
- Sí existen cola/resolución de reportes, inspect/where/goto/transfer/restore,
  anuncios y consulta de auditoría, con motivos obligatorios y límites.
- Sí existen inventario, equipo, consumibles, botín fijo de NPC, prototipos
  reutilizables, instancias durables, capacidad por slots/peso y
  rareza/`itemLevel` textual. También existen pilas fungibles derivadas,
  contenedores acotados, cantidades, consulta paginada, `give` atómico y las
  políticas `transferable`, `character_bound` y `non_transferable`. También
  existe trade bilateral confirmado mediante reservas durables, revisión y
  doble confirmación de objetos y moneda.
- Sí existen tablas enteras fixed+repeatable, procedencia durable, derechos de
  20 ticks, reparto de cohorte round-robin, `loot pass` y despawn al tick 120.
- No existen afijos, recompensas selladas, need/greed, master-loot o reparto por
  contribución. Las cajas y llaves pagadas quedan fuera.

Las fases 1, comunicación/grupo, 2, staff por capacidades, 3A, fundamento de
objetos, 3B, pilas/contenedores, 3C, entrega/políticas, y 3D, trade confirmado,
están implementadas. También está implementada 4A, botín repetible con derechos
y procedencia. La continuación 4B reserva afijos y recompensas selladas ganadas
jugando.

## Estado verificado en el código

### Conversación y seguridad social

`SocialAction` ofrece `Say`, `PartySay`, `Tell`, `Reply`, listado/alta/baja/envío
de canal, `Who`, `Help`, ignore/block, reportes, mute/ban, reversión de
sanciones y cambio de rol. `SocialChannel` contiene `Announcement`, `Say`,
`Tell`, `Party`, `General`, `Newbie` y `Staff`.

- `say` se entrega a otros personajes conectados en la misma sala;
- `tell` se entrega a un personaje conectado, aunque esté en otra sala;
- `reply` conserva en memoria el ID interno del último remitente cuyo `tell`
  se entregó y exige que siga conectado; no es correo durable;
- `party say` se entrega solo a los demás miembros conectados del grupo;
- `general`, `newbie` y `staff` exigen suscripción durable; `staff` solo está
  disponible para perfiles `moderator`, `administrator` o `implementor`, tanto
  al unirse o enviar como al recibir; `helper` y `builder` no acceden;
- un `tell` bloqueado o ignorado se confirma igual que uno entregado para no
  filtrar el estado privado del receptor;
- el texto y el destinatario están acotados, y el chat tiene rate limit;
- la entrega usa el router directo y los buzones acotados por sesión;
- reportes, relaciones, sanciones y suscripciones sobreviven a un reinicio.

SQLite y PostgreSQL implementan actualmente el esquema v11. Conserva
`social_channel_subscriptions` de v9 y añade perfiles, capacidades, resolución
cerrada de reportes y auditoría append-only de v10; v11 amplía a 2 MiB el
snapshot actual sin relajar el límite legacy v1–v5 de 1 MiB. El traslado sin
conexión también los copia. La evidencia histórica PostgreSQL real sigue
acreditando v8 y no se presenta como aceptación v11.

Por tanto, `tell` sirve como mensaje privado online clásico, pero no debe
presentarse como telepatía: no consume una habilidad o recurso, no aplica
alcance, sentidos, interferencias ni condiciones de mundo, y no llega a un
personaje desconectado.

### Parties

`PartyAction` implementa `Create`, `Invite`, `Accept`, `Revoke`, `Kick`,
`Transfer`, `Disband`, `Leave` y `Status`.

- máximo de seis miembros;
- solo el líder invita, revoca, expulsa, transfiere y disuelve;
- máximo de doce invitaciones pendientes;
- una invitación expira tras veinte ticks lógicos;
- la transferencia explícita exige otro miembro; si el líder sale, el
  liderazgo pasa de forma determinista al primer miembro restante;
- un miembro desconectado sigue perteneciendo al grupo y aparece como
  acompañante ausente;
- el snapshot durable conserva parties e invitaciones.

Accept, revoke, kick y transfer resuelven el nombre de un personaje conectado.
La persistencia de membresía offline no implica que esas operaciones dirigidas
puedan gestionar a un objetivo desconectado.

No hay seguimiento, asistencia/rescate, selección general de aliados ni
crédito de misión/XP compartido. La única política de botín de grupo actual es
round-robin sobre una cohorte próxima congelada; no hay need/greed, master-loot
o contribución.

### Staff y moderación

La autorización ejecutable es de cuenta y comprueba una capacidad concreta.
Los roles `Player`, `Moderator`, `Admin` solo conservan el tier legado de
moderación y percepción; nunca son suficientes para ejecutar un comando ni
abren por sí solos el canal `staff`. Ese canal exige perfil `moderator`,
`administrator` o `implementor`; `helper` y `builder` no acceden. El primer
registro de una base nueva conserva Admin como bootstrap legado y recibe
perfil `implementor` con las doce capacidades. Los Admin migrados desde v9
reciben `administrator`. Existe recuperación offline de emergencia para una
instalación sin admin.

Los perfiles cerrados y su bundle inicial son:

| Perfil | Bundle inicial |
| --- | --- |
| `helper` | `reports.view`, `reports.resolve` |
| `builder` | Ninguna capacidad implícita; se asigna explícitamente lo necesario. |
| `moderator` | Reportes más `moderation.mute`, `moderation.ban` |
| `administrator` | Las doce capacidades. |
| `implementor` | Las doce capacidades; rango operativo excepcional. |

El `lore_title` opcional puede presentar “Angel”, “Semidiós”, “Dios” u otro
título del mundo, pero no concede autoridad. Las capacidades canónicas son:

```text
reports.view       reports.resolve
moderation.mute    moderation.ban
staff.assign       staff.inspect       staff.where
staff.goto         staff.transfer      staff.restore
staff.announce     staff.audit
```

Cambiar un perfil borra las asignaciones anteriores y siembra exactamente el
bundle revisado del perfil nuevo; las capacidades directas se aplican después,
de forma explícita. Tanto la asignación como las acciones dirigidas rechazan la
propia cuenta y cualquier objetivo de rango igual o superior.

Dentro del juego se pueden ejecutar `report`, `mute`, `unmute`, `ban`,
`unban`, `role` y la superficie staff de fase 2. Todos los cambios sensibles
exigen motivo. La cola solo contiene reportes abiertos; resolver requiere uno
de `dismissed`, `action_taken`, `no_action` y conserva resolución, motivo,
responsable y fecha.

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

Inspect/where/goto/transfer/restore resuelven un personaje conectado y no son
administración offline. Perfil/capacidad y resolución de reportes usan
identidad durable. Las mutaciones y su entrada de auditoría se escriben
atómicamente; `staff_audit` es append-only por restricciones y triggers en
ambos adaptadores. Siguen ausentes `load/spawn`, `purge`, `invis`, `holylight`,
`snoop`, `switch`, `force` y edición online. `tiamud-admin` permanece en la
capa de operador.

### Inventario y botín

WorldPack v6/AreaPack v5 separan `itemPrototypes` de
`initialItemInstances`. El prototipo contiene nombre, descripción, aliases,
equipo/ataque/curación/efecto, peso, coste de slot, rareza, nivel y políticas
cerradas. Cada instancia durable referencia un prototipo y comienza en una
sala, un `npc_spawn`, `staged` o dentro de otra instancia contenedor. En runtime
puede estar además en inventario, equipada, en poder de un NPC o consumida. Los
slots de equipo actuales son arma, cuerpo y abalorio.

El runtime crea instancias distintas del mismo prototipo con una secuencia
monotónica persistida. `ItemInstanceId`, `prototypeId` y spawn no salen del
servidor; Web, Telnet y GMCP reciben un ID opaco distinto por unidad, derivado
de una sal criptográficamente aleatoria nueva por sesión. Telnet lo muestra
como referencia y permite usarlo como selector exacto. Una pila visible usa la
referencia opaca de su primer miembro visible ordenado; las consultas textuales
ambiguas fallan en vez de escoger una unidad arbitraria. 4A añade procedencia
durable exacta: `authored_initial`, `legacy_snapshot`, `runtime_created` o
`npc_loot` con tabla/digest/entrada/tirada; esa procedencia nunca sale del
servidor.

La presentación de un objeto se autoriza por instancia visible, pero el
registro `entityMedia` se comparte por prototipo. La política
`roomPerception.hiddenItems` apunta a la instancia concreta; no es una regla de
visibilidad para todas las unidades del prototipo.

Cada personaje tiene capacidad por slots y peso. El peso suma inventario y
equipo y todos sus descendientes; los slots cuentan solo raíces de inventario,
agrupadas por pila. Un contenedor `unique` no consumible cuenta slots de hijos
directos agrupados y peso de todos sus descendientes. El grafo rechaza ciclos y
una quinta arista. Equipar, desequipar, reemplazar, tomar, soltar, introducir y
extraer prevalidan el estado prospectivo completo antes de mutar.

Las pilas `fungible` se derivan por ubicación/prototipo/vínculo, ordenan
unidades por ID y se dividen por `maxStackUnits`; no se persiste `quantity` ni
un ID de pila. `take`/`drop`/`put`/`get`/`give` mueven 1..=256 unidades y
`contents` pagina 20 grupos.
La rareza cerrada, `itemLevel`, peso unitario/total, cantidad, slots y capacidad
se muestran como texto; Web usa `container.contents`, Telnet texto equivalente
y GMCP `Char.Items.List` con ubicación `container`. Las vistas añaden política,
estado ligado y `canGive`, sin exponer el dueño durable.

El propietario efectivo se obtiene en la raíz del grafo. Al adquirir desde
sala/NPC/staging se liga recursivamente cada descendiente `character_bound` al
personaje; `non_transferable` restringe el cambio sin persistir un dueño. Un
subárbol solo puede cambiar de personaje o volver al mundo si todas sus
instancias son `transferable`, cerrando el bypass mediante contenedores.
`give [cantidad] <objeto> to <personaje>` solo acepta raíces portadas, otro
personaje conectado en la misma sala y capacidad bilateral. Ignore del
receptor y block bilateral lo impiden con un error genérico. El cambio,
snapshot y `ItemsGiven` se confirman o revierten juntos.

`trade` mantiene objetos y moneda en sus autoridades y los reserva en el
snapshot. Admite hasta 64 negociaciones, una por personaje, 8 líneas por lado
y 128 raíces transferibles entre ambos. Los dos participantes deben seguir
conectados, juntos, fuera de combate y permitidos por ignore/block. Cada cambio
material incrementa una revisión y limpia confirmaciones; dos identidades
distintas deben confirmar la misma revisión y digest. La liquidación construye
ambos inventarios y saldos finales antes de confirmar todo en un único commit.
Separación, combate, derrota, desconexión, cambio social, expiración o reinicio
cancelan de forma cerrada. Web, Telnet y `Char.Trade 1` GMCP proyectan ofertas,
reservas y saldo gastable con IDs opacos por sesión.

El botín fijo continúa como instancia inicial ubicada en `npc_spawn`, pero 4A
añade una `lootSource` repetible con una `lootTable` de garantizados y pools
ponderados enteros. Cada fuente se resuelve una sola vez por derrota mediante
rejection sampling. Fijos y generables suman como máximo 64 raíces; tirada,
RNG, contador, procedencia, derechos, NPC, XP, eventos y snapshot se confirman
o revierten juntos. Si falta capacidad, lo generado se suprime de forma
durable sin bloquear la derrota ni permitir reroll.

`credited_character` reserva las raíces al iniciador. `party_round_robin`
congela una cohorte de hasta seis personajes conectados, vivos y próximos,
revalida a los acompañantes en la derrota y avanza un cursor durable. El
beneficiario puede usar `loot pass`; la raíz es exclusiva durante 20 ticks,
pública después y se retira al tick 120. Web, Telnet y GMCP solo muestran
`yours` con ticks restantes, `reserved` sin identidad, o público; las pilas se
parten por ese acceso.

`WorldRuntimeSnapshot` v6 persiste instancias, contención, contadores, vínculo,
trade, RNG, procedencia, encuentros, derechos y cursores. Las lecturas v1–v5
usan DTO exactos bajo 1 MiB, migran con procedencia legacy y rechazan campos v6;
el snapshot actual admite hasta 2 MiB. SQLite y PostgreSQL confirman esa única
autoridad en la transacción de esquema v11, sin tabla espejo de loot/escrow.
Al restaurar se conservan nuevas instancias authored sin resucitar runtime
ausente. La barrera valida por lotes propietarios, vínculos, cohortes y
beneficiarios; cancela encuentros abiertos antes de readiness, pero conserva
los derechos de suelo ya creados.

Las escenas densas conservan una ventana semántica determinista de 14 KiB en
Web y 48 KiB en Telnet, con prioridad propia→pública→reservada y aviso de
reducción. Cada batch proyecta como máximo 16 eventos y un exceso se coalesce
con snapshot autoritativo; esto no introduce replay general.

## Qué muestran los MUD clásicos

Las referencias oficiales de CircleMUD documentan familias de funciones como
grupo y `gsay`, comunicación pública, seguimiento/asistencia/rescate, correo y
tablones, y una superficie amplia de comandos de wizard. Sus niveles mínimos
de comando también separan parte de la autoridad inmortal del avance mortal:

- [índice oficial del código fuente de CircleMUD](https://www.circlemud.org/pub/jelson/CircleMUD/3.x/uncompressed/current/src/);
- [referencia cruzada oficial de funciones](https://www.circlemud.org/cdp/cxref/comm.c.html);
- [manual oficial de comandos de wizard](https://www.circlemud.org/pub/CircleMUD/3.x/uncompressed/circle30bpl21/doc/wizhelp.pdf);
- [notas oficiales sobre canales y niveles mínimos](https://www.circlemud.org/cdp/wtfaq/chap3-1.html);
- [notas oficiales sobre niveles inmortales adicionales](https://www.circlemud.org/cdp/wtfaq/level.html).

Estas fuentes sirven para identificar categorías, no para copiar código,
textos de ayuda o decisiones de seguridad. Tiamud es una implementación limpia
y conserva sus propios límites de licencia, accesibilidad, autoridad,
persistencia y single writer.

## Comunicación implementada y ampliaciones futuras

La fase 1 conserva una sola política social; ningún canal está implementado de
forma independiente en WebSocket, Telnet o la interfaz. La futura telepatía
deberá entrar por esa misma política.

```mermaid
flowchart LR
    accTitle: Enrutamiento actual y futuro de las comunicaciones de Tiamud
    accDescr: Las acciones de texto implementadas entran en una política social común que calcula destinatarios, aplica bloqueos y límites y entrega solo a buzones autorizados. La telepatía sigue siendo una ampliación futura que deberá reutilizar la misma política.

    Text[PlayerAction social] --> Policy[Política social común]
    Ability[Habilidad futura de telepatía] -.-> Effect[Efecto autorizado]
    Effect --> Policy
    Policy --> Audience{Ámbito}
    Audience --> Room[Sala]
    Audience --> Party[Party]
    Audience --> Direct[Directo]
    Audience --> Channel[Canal suscrito]
    Policy --> Safety[Ignore · block · mute<br/>rate limit · permisos]
    Room --> Delivery[Buzones acotados por sesión]
    Party --> Delivery
    Direct --> Delivery
    Channel --> Delivery
    Safety --> Delivery
```

### Fase 1 entregada

1. Implementado `party say <texto>` con alias `gtell`/`gsay`, usando la
   membresía durable actual.
2. Implementado `reply <texto>` con el ID interno en memoria del último
   remitente válido, no un nombre ambiguo ni un ID estable expuesto.
3. Implementados revocar invitación, expulsar, transferir y disolver.
4. Implementados los canales suscritos `newbie`, `general` y `staff`, con
   permisos, rate limit, mute, ignore/block y entrega mediante buzones
   acotados. La suscripción persiste en ambos adaptadores; `staff` exige perfil
   `moderator`, `administrator` o `implementor`.
5. Pendiente `shout`/`yell` como propagación espacial definida por el mundo y
   acotada, no como otro nombre para el canal global.
6. Pendientes emotes/sociales con plantillas estrictas que nunca conviertan
   texto del jugador en ANSI, HTML, MXP o instrucciones evaluables.

Web usa acciones estructuradas y representa los siete valores de
`SocialMessage`; Telnet comparte los comandos de texto y proyecta cada entrega
como texto y como `Comm.Channel.Text` cuando GMCP `Comm` está negociado. Esta
paridad no convierte GMCP en una segunda autoridad.

El correo offline, tablones y guild/clan chat pueden seguir después. Requieren
retención, límites, búsqueda/moderación, eliminación y reglas de privacidad
propias.

### Telepatía

La telepatía debe ser data-driven y opcional por mundo. Una definición mínima
necesita:

- habilidad o efecto que la habilita;
- destinatario directo o party;
- coste y cooldown;
- alcance o ausencia explícita de alcance;
- sentidos/estados que permiten emitir y recibir;
- interferencias y zonas que la bloquean;
- una proyección textual completa para ambos transportes;
- los mismos límites sociales, ignore, block, mute y auditoría aplicables al
  resto de comunicaciones.

No se debe disfrazar `tell` como telepatía porque eso impediría a cada
WorldPack decidir su ficción y sus reglas.

## Staff: rango visible, capacidad real

Los niveles de personaje y la autoridad administrativa deben seguir siendo
dos ejes independientes.

```mermaid
flowchart TB
    accTitle: Separación objetivo entre progresión y autoridad de staff
    accDescr: La experiencia solo cambia la ficha jugable. Una designación administrativa separada concede capacidades acotadas, puede mostrar un título de ambientación y produce auditoría para cada acción sensible.

    subgraph Play[Progresión jugable]
        XP[XP] --> Level[Nivel]
        Level --> Class[Clase y talentos]
        Class --> Power[Poder del personaje]
    end

    subgraph Staff[Autoridad asignada]
        Assign[Designación explícita] --> Profile[Perfil de capacidades]
        Assign --> Lore[Título visible opcional]
        Profile --> Bundle[Bundle inicial revisado]
        Assign --> Capability[Asignación directa]
        Bundle --> Capability
        Capability --> Command[Comando autorizado]
        Command --> Reason[Motivo y límites]
        Reason --> Audit[(Auditoría durable)]
    end
```

Los perfiles canónicos son identificadores operativos no localizados. “Angel”,
“Semidiós” o “Dios” caben como `lore_title`, nunca como autorización. La
comprobación usa el conjunto efectivo de capacidades y la jerarquía negativa
de perfiles; rechaza objetivos iguales o superiores aunque exista la capacidad
funcional. Cada comando de mundo implementado exige motivo, limita el alcance,
opera sobre un personaje conectado y deja auditoría durable.

`force`, `snoop` y `switch` tienen riesgos de abuso y privacidad; no son
requisitos del baseline. `load/spawn` y `purge` pueden romper la economía o la
persistencia si no trabajan con instancias y transacciones. El apagado, backup
y migración deben seguir en la capa operativa salvo una necesidad medida.

La construcción online tampoco debe mutar definiciones vivas arbitrariamente.
Un futuro OLC debería producir un borrador AreaPack, ejecutar el validador,
publicar un WorldPack inmutable y usar activación/rollback controlados.

## Inventario, tiers e instancias

“Tier de inventario” puede significar rareza del objeto o capacidad de carga.
Son sistemas distintos y deben modelarse por separado.

```mermaid
flowchart LR
    accTitle: Inventario implementado y ampliaciones de transferencia y botín
    accDescr: Cada unidad conserva identidad durable; pilas, contenedores, políticas, trade, tablas repetibles, procedencia y derechos ya son autoritativos. Los afijos y recompensas selladas quedan para 4B.

    Prototype[ItemPrototype implementado<br/>nombre · reglas · rareza · peso] --> Instance[ItemInstance implementada<br/>ID durable · prototypeId · ubicación]
    Sequence[(Secuencia persistida)] --> Instance
    Instance --> Current[Inventario / equipo / NPC / sala / contenedor]
    Current --> Stack[Pila derivada<br/>sin identidad durable]
    Current --> Container[Contenedor acotado<br/>slots + peso recursivo]
    Current --> Give[Give atómico<br/>solo subárbol transferible]
    Policy[Política + vínculo durable] --> Give
    Give --> Trade[Trade confirmado<br/>reserva + doble confirmación]

    Prototype --> Table[LootTable validada<br/>fixed + weighted]
    RNG[(RNG persistido)] --> Roll[Tirada entera determinista]
    Table --> Roll
    Roll --> Generated[Instancia nueva + procedencia]
    Generated --> Rights[Derecho 20 ticks]
    Cohort[Cohorte congelada] --> RR[Round-robin + pass]
    RR --> Rights
    Rights --> Public[Público]
    Public --> Despawn[Despawn tick 120]
    Table -. fase 4B .-> Sealed[Selladas + afijos]
```

### Fundamento y secuencia

1. Implementado: `ItemPrototype`, contenido authored estable y reutilizable.
2. Implementado: `ItemInstance`, ID durable único, `prototypeId` y ubicación;
   el snapshot y rollback incluyen la secuencia creadora.
3. Implementado: inventario acotado por slots/peso, contenedores con capacidad
   y profundidad máximas, y pilas derivadas que agrupan unidades fungibles por
   ubicación/prototipo sin persistir cantidad.
4. Implementado: `give` atómico para casos simples y políticas declaradas
   `transferable`, `character_bound` y `non_transferable`, aplicadas al
   propietario efectivo y al subárbol completo.
5. Implementado: intercambio confirmado por ambas partes para ofertas de
   objetos y moneda, con reserva durable acotada e invalidación ante cualquier
   cambio material.
6. Implementado: tablas enteras fixed+repeatable, procedencia, ventana
   exclusiva de 20 ticks, round-robin de cohorte y `loot pass`; publicación y
   despawn ocurren a 20/120 ticks.
7. Pendiente 4B: afijos y recompensas selladas obtenidas jugando.

### Rareza y nivel de objeto

La rareza cerrada `common|uncommon|rare|epic|legendary` ya forma parte del
prototipo y se muestra como texto en navegador y Telnet/GMCP; nunca depende solo
del color.

`itemLevel`, calidad y afijos son ejes diferentes. Deben tener límites authored
por slot/tier/nivel y aritmética saturada. Una rareza alta no puede saltarse
requisitos, capacidades o validación económica.

### Tablas y recompensas selladas

4A implementa tablas con pesos enteros acotados y suma comprobada, cantidades y
tiradas limitadas, `nothing`, RNG persistido con rejection sampling,
procedencia durable y commit atómico. No incluye tablas anidadas, condiciones,
afijos, reglas de únicos, pity ni scripts de recompensa. El peor caso fijo más
generado se valida a 64 raíces por derrota.

4B podrá añadir recompensas selladas conseguidas jugando y afijos. Abrir una
deberá prevalidar tirada, vínculo y capacidad antes de consumir contenedor, RNG
o contador, confirmar recompensa y consumo juntos y mostrar probabilidades
exactas. Las loot boxes, llaves y tiradas pagadas quedan fuera del baseline:
añaden incentivos y riesgos legales/éticos innecesarios para el motor. Si algún
día se plantea monetización, es preferible vender cosméticos conocidos o apoyo
de precio fijo.

## Otros sistemas MUD candidatos

| Sistema | Estado actual | Cuándo considerarlo |
| --- | --- | --- |
| Emotes/sociales | Ausente. | Con el router social ampliado y plantillas seguras. |
| Mail y tablones | Ausente. | Después de definir retención, cuotas, moderación y privacidad. |
| Guilds/clanes | Ausente. | Tras estabilizar parties, canales y autoridad comunitaria. |
| Puertas y llaves | Ausentes como sistema general; los contenedores de inventario sí existen. | Cuando el estado abierto/cerrado, acceso y llaves tenga autoría/persistencia explícita. |
| Give | Implementado para raíces portadas, misma sala, capacidad bilateral y subárbol enteramente transferible. | Ampliarlo solo si se diseña entrega remota/offline con política explícita. |
| Trade/auction | Trade directo confirmado implementado; subasta ausente. | Subastas solo tras definir mercado offline, retención, cuotas y abuso económico. |
| Crafting | Ausente. | Cuando haya prototipos repetibles y sinks económicos. |
| Housing | Ausente. | Cuando el contenido necesite instancias persistentes privadas. |
| Pets/mounts | Ausente. | Después de followers y reglas de party/NPC. |
| Duelo/PvP | Ausente. | Solo con consentimiento, seguridad, muerte/recompensa y moderación diseñados. |
| Facciones/reputación/idiomas | Ausente. | Como contenido data-driven cuando exista una necesidad narrativa. |
| Remort/rebirth | Ausente. | No prioritario mientras v3 aún necesita balance y migraciones. |
| OLC in-game | Ausente. | Como staging/validación/publicación, nunca mutación libre del runtime. |

## Plan por fases y aceptación

### Fase 1 — comunicación y grupo — implementada

- chat de grupo, `reply`, revocación/expulsión/transferencia/disolución;
- canales `newbie`/`general`/`staff` suscritos y moderados;
- paridad navegador, texto Telnet y GMCP;
- cobertura de ignore/block/mute, límites, desconexión, perfil de `staff` y cero
  fuga entre grupos.

### Fase 2 — staff por capacidades — implementada

- perfiles y capacidades separados de progresión;
- cola y resolución cerrada de reportes y razones obligatorias;
- inspect/where/goto/transfer/restore/announce/audit acotados;
- autorización negativa, jerarquía, persistencia v10 y auditoría append-only
  en SQLite/PostgreSQL;
- acciones estructuradas Web y texto/announcement Telnet+GMCP equivalentes;
- ninguna autoridad obtenible mediante XP, clase, talento o WorldPack.

### Fase 3A — fundamento prototype/instance — implementada

- WorldPack v4/AreaPack v3, normalización v3/v2 y v2/v1 respectivamente;
- prototipos reutilizables, instancias durables y contador monotónico;
- snapshot v2, conversión explícita desde v1 y rollback exacto;
- slots/peso, rareza/`itemLevel` textual e IDs opacos en ambos transportes;
- en aquel corte, snapshot único en SQLite/PostgreSQL v10, sin tabla espejo.

### Fase 3B — pilas y contenedores — implementada

- WorldPack v5/AreaPack v4 y snapshot v3 con conversión explícita v1/v2/v3;
- pilas fungibles derivadas, sin cantidad ni identidad de pila durable;
- contenedores únicos con slots, peso recursivo, profundidad cuatro y ciclos
  rechazados antes de mutar;
- cantidades máximas 256, `contents` paginado a 20 grupos y selectores opacos;
- paridad de datos/acciones en Web, Telnet y `Char.Items.List` GMCP.

### Fase 3C — entrega y políticas — implementada

- `give` atómico, propietario efectivo y políticas de transferencia
  ejecutables;
- binding durable en snapshot v4, migración cerrada desde v1-v3 y paridad
  Web/Telnet/GMCP;
- ignore/block, capacidad bilateral, replay/rollback y proyección opaca.

### Fase 3D — intercambio confirmado — implementada

- reserva durable sin custodia separada, doble confirmación e invalidación ante
  cambios;
- límites globales/por participante, expiración, política social y cancelación
  ante separación, combate, derrota, desconexión o reinicio;
- liquidación simultánea de objetos/moneda, snapshot v5 y paridad
  Web/Telnet/GMCP.

El contrato aceptado e implementado está en
[ADR 0012](../decisions/0012-confirmed-trade-reservations.md).

### Fase 4A — botín repetible, derechos y procedencia — implementada

- WorldPack v6/AreaPack v5/WorldProject v2 con una tabla y fuente canónicas;
- snapshot v6 y esquema SQL v11; legacy v1–v5 limitado a 1 MiB y v6 a 2 MiB;
- pesos enteros, rejection sampling y resolución exactly-once de botín fijo más
  repetible, con máximo combinado de 64 raíces;
- procedencia durable, cohorte congelada, round-robin, `loot pass`, exclusividad
  20 ticks y despawn 120;
- supresión por capacidad sin bloquear derrota/XP ni permitir reroll;
- arranque que valida identidades y cancela encuentros abiertos sin borrar
  derechos resueltos;
- privacidad y acciones equivalentes en Web/Telnet/GMCP, con ventanas
  semánticas 14/48 KiB y batches/coalescing de 16 eventos.

El contrato aceptado e implementado está en
[ADR 0013](../decisions/0013-repeatable-loot-rights-and-provenance.md).

### Fase 4B — selladas y afijos — pendiente

- recompensas selladas obtenidas jugando, con probabilidades exactas;
- simulación de tirada, vínculo y capacidad antes del consumo atómico;
- afijos acotados por rareza/`itemLevel` y economía;
- pruebas deterministas de probabilidades y límites.

Need/greed, master-loot y reparto por contribución no forman parte de 4B sin
diseño adicional. Cajas, llaves y tiradas pagadas permanecen fuera.

### Fase 5 — comunidad y contenido — pendiente

- emotes, mail/boards y guilds según necesidades observadas;
- crafting/auction/housing solo con sinks, cuotas y moderación;
- OLC únicamente sobre el pipeline de artefactos revisados.

Cada fase debe preservar el single writer, una sola persistencia seleccionada,
rollback exacto, límites de frame/cola, percepción antes de protocolo, IDs
opacos y equivalencia funcional entre browser y Telnet.
