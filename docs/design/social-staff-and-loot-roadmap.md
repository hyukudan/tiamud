# Hoja de ruta de sistemas sociales, staff e inventario/botín

Estado: auditoría de implementación y diseño de continuación, 2026-07-16.

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
  contenedores acotados, cantidades y consulta paginada.
- No existen `give`/trade, políticas ligadas ejecutables, derechos o reparto de
  botín, afijos ni tablas de botín aleatorias.

Las fases 1, comunicación/grupo, 2, staff por capacidades, 3A, fundamento de
objetos, y 3B, pilas/contenedores, están implementadas. La secuencia pendiente
es: transferencias y trade confirmado; propiedad y reparto del botín; y después, drops
repetibles, afijos y tablas aleatorias.

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

SQLite y PostgreSQL implementan actualmente el esquema v10. Conserva
`social_channel_subscriptions` de v9 y añade perfiles, capacidades, resolución
cerrada de reportes y auditoría append-only; el traslado sin conexión también
los copia. La evidencia histórica PostgreSQL real sigue acreditando v8 y no se
presenta como aceptación v10.

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

No hay seguimiento, asistencia/rescate, selección general de aliados, crédito
de misión/XP compartido ni política de botín.

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

WorldPack v5/AreaPack v4 separan `itemPrototypes` de
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
ambiguas fallan en vez de escoger una unidad arbitraria. 3B no almacena origen
o procedencia durable: la ubicación inicial authored no sustituye ese futuro
registro de fase 4.

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

Las pilas `fungible` se derivan por ubicación/prototipo, ordenan unidades por
ID y se dividen por `maxStackUnits`; no se persiste `quantity` ni un ID de pila.
`take`/`drop`/`put`/`get` mueven 1..=256 unidades y `contents` pagina 20 grupos.
La rareza cerrada, `itemLevel`, peso unitario/total, cantidad, slots y capacidad
se muestran como texto; Web usa `container.contents`, Telnet texto equivalente
y GMCP `Char.Items.List` con ubicación `container`.

El botín fijo actual es una instancia inicial ubicada en `npc_spawn`. El
validador exige prototipo, instancia, spawn y ubicación coherentes. La unidad
se entrega al NPC al cargar y se transfiere al derrotarlo. Esto sigue siendo
botín fijo y determinista, no una tabla aleatoria repetible.

`WorldRuntimeSnapshot` v3 persiste instancias, contención y contador; las
lecturas v1/v2 usan DTO y conversiones explícitos dirigidos por versión. SQLite
y PostgreSQL confirman ese snapshot único en la
transacción existente de esquema v10. Al restaurar se conservan nuevas
instancias iniciales del WorldPack y se superpone el estado durable, sin
recuperar instancias runtime ausentes del snapshot. La barrera de arranque
valida propietarios siguiendo contenedores hasta su raíz, equipo, grafo y
capacidad de personajes conectados o
desconectados mediante agregados acotados que descarta después. No hay tabla
espejo ni esquema v11.

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
    accDescr: Cada unidad conserva identidad durable; pilas y contenedores ya forman una vista y un grafo acotados. Transferencias, derechos y tablas siguen como fases posteriores.

    Prototype[ItemPrototype implementado<br/>nombre · reglas · rareza · peso] --> Instance[ItemInstance implementada<br/>ID durable · prototypeId · ubicación]
    Sequence[(Secuencia persistida)] --> Instance
    Instance --> Current[Inventario / equipo / NPC / sala / contenedor]
    Current --> Stack[Pila derivada<br/>sin identidad durable]
    Current --> Container[Contenedor acotado<br/>slots + peso recursivo]

    Prototype -. fase 4 .-> Table[LootTable validada]
    RNG[(RNG persistido)] --> Roll[Tirada entera determinista]
    Table --> Roll
    Roll --> Generated[Instancia nueva + procedencia]
    Generated --> Rights[Elegibilidad y propietario]
    Rights --> Future[Reparto]
    Current -. fase 3C/3D .-> Trade[Dar / intercambio confirmado]
```

### Fundamento y secuencia

1. Implementado: `ItemPrototype`, contenido authored estable y reutilizable.
2. Implementado: `ItemInstance`, ID durable único, `prototypeId` y ubicación;
   el snapshot y rollback incluyen la secuencia creadora.
3. Implementado: inventario acotado por slots/peso, contenedores con capacidad
   y profundidad máximas, y pilas derivadas que agrupan unidades fungibles por
   ubicación/prototipo sin persistir cantidad.
4. Pendiente: transferencia atómica, con `give` para casos simples e intercambio confirmado
   por ambas partes para objetos valiosos. Objetos de quest, ligados o no
   transferibles deben declararlo. En 3B solo se acepta `transferable`;
   `character_bound` y `non_transferable` se rechazan como valores reservados
   hasta que 3C ejecute su semántica.
5. Pendiente: derechos de botín con ventana exclusiva y política inicial determinista, por
   ejemplo round-robin, antes de dejarlo libre en la sala.

### Rareza y nivel de objeto

La rareza cerrada `common|uncommon|rare|epic|legendary` ya forma parte del
prototipo y se muestra como texto en navegador y Telnet/GMCP; nunca depende solo
del color.

`itemLevel`, calidad y afijos son ejes diferentes. Deben tener límites authored
por slot/tier/nivel y aritmética saturada. Una rareza alta no puede saltarse
requisitos, capacidades o validación económica.

### Tablas y cofres aleatorios

Una tabla de botín segura necesita:

- pesos enteros acotados y suma comprobada;
- número mínimo/máximo de tiradas y cantidades acotadas;
- condiciones con la gramática declarativa existente, no scripts arbitrarios;
- reglas de únicos y duplicados;
- RNG persistido para que un reinicio no permita repetir o cambiar la tirada;
- creación, asignación, evento de dominio y snapshot en la misma transacción;
- procedencia durable para diagnóstico y economía;
- probabilidades visibles para cualquier contenedor aleatorio.

Los cofres conseguidos jugando pueden ser contenido normal si no se venden
llaves, tiradas ni moneda premium y si sus probabilidades son transparentes.
Las loot boxes de pago quedan fuera del baseline: añaden incentivos y riesgos
legales/éticos que no son necesarios para el motor. Si algún día se plantea
monetización, es preferible vender cosméticos conocidos o apoyo de precio fijo.

## Otros sistemas MUD candidatos

| Sistema | Estado actual | Cuándo considerarlo |
| --- | --- | --- |
| Emotes/sociales | Ausente. | Con el router social ampliado y plantillas seguras. |
| Mail y tablones | Ausente. | Después de definir retención, cuotas, moderación y privacidad. |
| Guilds/clanes | Ausente. | Tras estabilizar parties, canales y autoridad comunitaria. |
| Puertas y llaves | Ausentes como sistema general; los contenedores de inventario sí existen. | Cuando el estado abierto/cerrado, acceso y llaves tenga autoría/persistencia explícita. |
| Give/trade/auction | Ausente. | Tras propiedad durable, operación atómica y antifraude básico. |
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
- snapshot único en SQLite/PostgreSQL v10, sin tabla espejo ni esquema v11.

### Fase 3B — pilas y contenedores — implementada

- WorldPack v5/AreaPack v4 y snapshot v3 con conversión explícita v1/v2/v3;
- pilas fungibles derivadas, sin cantidad ni identidad de pila durable;
- contenedores únicos con slots, peso recursivo, profundidad cuatro y ciclos
  rechazados antes de mutar;
- cantidades máximas 256, `contents` paginado a 20 grupos y selectores opacos;
- paridad de datos/acciones en Web, Telnet y `Char.Items.List` GMCP.

### Fase 3C/3D — transferencias — pendiente

- `give` atómico y políticas de transferencia ejecutables;
- intercambio con escrow, doble aceptación e invalidación ante cambios.

### Fase 4 — propiedad y botín aleatorio — pendiente

- elegibilidad y distribución de party;
- procedencia durable y afijos con presupuestos de rareza/`itemLevel`;
- tablas enteras validadas y tirada transaccional restart-safe;
- probabilidades visibles, pruebas estadísticas deterministas y límites de
  economía;
- cofres solo como recompensa de juego en el baseline.

### Fase 5 — comunidad y contenido — pendiente

- emotes, mail/boards y guilds según necesidades observadas;
- crafting/auction/housing solo con sinks, cuotas y moderación;
- OLC únicamente sobre el pipeline de artefactos revisados.

Cada fase debe preservar el single writer, una sola persistencia seleccionada,
rollback exacto, límites de frame/cola, percepción antes de protocolo, IDs
opacos y equivalencia funcional entre browser y Telnet.
