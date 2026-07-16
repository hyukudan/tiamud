# Hoja de ruta de sistemas sociales, staff e inventario/botín

Estado: auditoría de implementación y diseño de continuación, 2026-07-16.

Este documento distingue lo que Tiamud implementa hoy de los sistemas que son
habituales en algunos MUD clásicos o convenientes para un mundo persistente
moderno. No convierte cada antecedente histórico en un requisito. El código y
las pruebas siguen siendo la autoridad sobre comportamiento actual; la
[matriz de completitud](../project/COMPLETION_MATRIX.md) conserva la autoridad
sobre la aceptación de la especificación histórica.

## Resumen ejecutivo

- Sí existen parties persistentes: crear, invitar, aceptar, salir y consultar
  el estado, con un máximo de seis miembros.
- No existe chat de party, `reply`, canales globales, correo offline ni una
  habilidad de telepatía.
- Sí existen los roles de cuenta `Player`, `Moderator` y `Admin`, separados de
  la progresión del personaje. Un Admin asigna los roles; la XP nunca los
  concede.
- Sí existen comandos de moderación dentro del juego: reportar, silenciar,
  expulsar, revertir sanciones y cambiar roles según autoridad.
- No existe una jerarquía de Angel/Semidiós/Dios ni una suite de comandos de
  inmortal para inspeccionar o modificar el mundo.
- Sí existen inventario, equipo, consumibles y botín fijo de NPC.
- No existen rareza/tier, peso, capacidad, pilas, contenedores, comercio,
  objetos repetibles ni tablas de botín aleatorias.

La secuencia recomendada es: comunicación y reglas de party; autoridad de
staff por capacidades; prototipos e instancias de objetos; propiedad y reparto
del botín; y, solo después, rareza y tablas aleatorias.

## Estado verificado en el código

### Conversación y seguridad social

`SocialAction` ofrece `Say`, `Tell`, `Who`, `Help`, ignore/block, reportes,
mute/ban, reversión de sanciones y cambio de rol. `SocialChannel` solo contiene
`Say` y `Tell`.

- `say` se entrega a otros personajes conectados en la misma sala;
- `tell` se entrega a un personaje conectado, aunque esté en otra sala;
- un `tell` bloqueado o ignorado se confirma igual que uno entregado para no
  filtrar el estado privado del receptor;
- el texto y el destinatario están acotados, y el chat tiene rate limit;
- la entrega usa el router directo y los buzones acotados por sesión;
- reportes, relaciones y sanciones sobreviven a un reinicio.

Por tanto, `tell` sirve como mensaje privado online clásico, pero no debe
presentarse como telepatía: no consume una habilidad o recurso, no aplica
alcance, sentidos, interferencias ni condiciones de mundo, y no llega a un
personaje desconectado.

### Parties

`PartyAction` implementa `Create`, `Invite`, `Accept`, `Leave` y `Status`.

- máximo de seis miembros;
- solo el líder invita;
- máximo de doce invitaciones pendientes;
- una invitación expira tras veinte ticks lógicos;
- al salir el líder, el liderazgo pasa al primer miembro restante;
- un miembro desconectado sigue perteneciendo al grupo y aparece como
  acompañante ausente;
- el snapshot durable conserva parties e invitaciones.

No hay expulsión, disolución explícita, revocación de invitación, chat privado,
seguimiento, asistencia/rescate, selección general de aliados, crédito de
misión/XP compartido ni política de botín.

### Staff y moderación

La autoridad actual es de cuenta y tiene tres valores ordenados: `Player`,
`Moderator` y `Admin`. Moderator y Admin pueden moderar únicamente roles
inferiores; solo Admin puede cambiar roles. El primer registro de una base
nueva recibe Admin y deja una auditoría de bootstrap. Existe recuperación
offline de emergencia para una instalación que se quede sin administrador.

Dentro del juego se pueden ejecutar `report`, `mute`, `unmute`, `ban`,
`unban` y `role`. Las sanciones y los cambios de rol son durables y auditados.
No existen `goto`, `transfer`, `restore`, `load/spawn`, `purge`, `stat`,
`invis`, `holylight`, `wiznet`, `snoop`, `switch`, `force` ni edición online
del mundo. `tiamud-admin` es una herramienta de operador fuera del juego.

### Inventario y botín

Un objeto actual del WorldPack es simultáneamente una definición y una
instancia única identificada por su ID estable. Puede estar staged, en una
sala, en un inventario, equipado, en posesión de un NPC o consumido. Las reglas
añaden opcionalmente un slot de equipo, ataque, curación, consumo y un efecto
acotado. Los slots actuales son arma, cuerpo y abalorio.

Un prototipo de NPC declara una lista fija de IDs de objetos. El validador
exige que existan, no se repitan y no se asignen a más de un spawn. Esos
objetos se entregan al NPC al cargar el mundo y se transfieren al derrotarlo.
Esto es botín fijo y determinista, no una tabla aleatoria repetible.

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

## Comunicación objetivo

La ampliación debe conservar una sola política social. No conviene implementar
cada canal directamente en WebSocket, Telnet o la interfaz.

```mermaid
flowchart LR
    accTitle: Enrutamiento objetivo de las comunicaciones de Tiamud
    accDescr: Acciones de texto y efectos de telepatía autorizados entran en una política social común, que calcula destinatarios, aplica bloqueos y límites y entrega únicamente a buzones de sesión autorizados.

    Text[PlayerAction social] --> Policy[Política social común]
    Ability[Habilidad de telepatía] --> Effect[Efecto de comunicación autorizado]
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

### Primera entrega social

1. `party say <texto>` con alias documentados `gtell`/`gsay`, usando la
   membresía durable actual.
2. `reply <texto>` guardando un ID interno del último remitente válido, no un
   nombre ambiguo ni un ID estable expuesto al cliente.
3. Liderazgo: revocar invitación, expulsar, transferir y disolver.
4. Canales opt-in iniciales: ayuda/newbie, conversación general y staff. Cada
   canal necesita permiso, suscripción, rate limit, mute, ignore/block y
   entrega limitada; no debe existir un broadcast sin política.
5. `shout`/`yell` como propagación espacial authored y acotada, no como otro
   nombre para el canal global.
6. Emotes/sociales con plantillas estrictas que nunca conviertan texto del
   jugador en ANSI, HTML, MXP o instrucciones evaluables.

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
        Profile --> Command[Comando autorizado]
        Command --> Reason[Motivo y límites]
        Reason --> Audit[(Auditoría durable)]
    end
```

Una configuración podría mostrar perfiles como Angel, Semidiós, Dios e
Implementor, pero esos nombres no deben ser la autorización canónica del
motor. Un mundo no fantástico puede llamarlos Helper, Builder, Moderator y
Administrator. Las capacidades permiten que un builder publique borradores
sin poder banear, y que un moderator atienda reportes sin poder crear objetos.

| Perfil orientativo | Capacidades posibles | No concede por defecto |
| --- | --- | --- |
| Helper / Angel | Ayuda, canal newbie, clasificar reportes. | Sanciones, viaje o mutación del mundo. |
| Builder / Semidiós | Previsualizar y validar borradores de áreas asignadas. | Publicación directa, cuentas o moderación. |
| Moderator | Consultar cola de reportes, mute/ban de rangos inferiores, resolver casos. | Spawn, restore, roles o secretos de operación. |
| Administrator / Dios | Asignar perfiles, announce, inspect, viaje/transfer/restore controlados. | Acceso implícito al host, base de datos o secretos. |
| Implementor | Recuperación excepcional y cambio de política. | Uso cotidiano desde un personaje normal. |

Los primeros comandos de mundo deberían ser de diagnóstico y reversibles:
`staff inspect`, `staff where`, `staff goto`, `staff transfer`,
`staff restore`, `staff announce` y `staff audit`. Toda acción que afecte a
otro personaje debe exigir motivo, comprobar una capacidad concreta, rechazar
objetivos iguales o superiores, limitar el alcance y dejar auditoría durable.

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
    accTitle: Modelo objetivo de objetos repetibles y botín
    accDescr: Un prototipo authored define propiedades comunes. Una tabla validada y el RNG persistido crean una instancia durable con procedencia; la transacción la asigna a un propietario o contenedor y después permite equipo, comercio o consumo.

    Prototype[ItemPrototype<br/>nombre · reglas · rareza base] --> Table[LootTable validada]
    RNG[(RNG persistido)] --> Roll[Tirada entera determinista]
    Table --> Roll
    Roll --> Instance[ItemInstance<br/>ID opaco · prototypeId<br/>afijos · procedencia]
    Instance --> Rights[Elegibilidad y propietario]
    Rights --> Inventory[Inventario / contenedor]
    Inventory --> Equip[Equipar]
    Inventory --> Trade[Dar / intercambio confirmado]
    Inventory --> Sink[Consumir / vender / destruir]
```

### Fundamento obligatorio

1. `ItemPrototype`: contenido authored estable y reutilizable.
2. `ItemInstance`: ID durable único, `prototypeId`, origen, ubicación,
   propietario/eligibilidad y, cuando proceda, seed o afijos resueltos.
3. Inventario acotado por slots y peso; contenedores con capacidad y
   profundidad máximas. Las pilas solo agrupan instancias fungibles con los
   mismos metadatos.
4. Transferencia atómica: `give` para casos simples e intercambio confirmado
   por ambas partes para objetos valiosos. Objetos de quest, ligados o no
   transferibles deben declararlo.
5. Derechos de botín: ventana exclusiva y política inicial determinista, por
   ejemplo round-robin, antes de dejarlo libre en la sala.

### Rareza y nivel de objeto

Una rareza cerrada como common/uncommon/rare/epic/legendary puede servir para
presupuesto de poder, frecuencia y presentación, pero no debe ser solo un
color. El cliente y Telnet deben mostrar texto y significado. El WorldPack
puede localizar las etiquetas visibles sin cambiar la enum canónica.

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
| Puertas, llaves y contenedores | Ausente como sistema general. | Con instancias, profundidad acotada y persistencia de estado. |
| Give/trade/auction | Ausente. | Tras propiedad durable, operación atómica y antifraude básico. |
| Crafting | Ausente. | Cuando haya prototipos repetibles y sinks económicos. |
| Housing | Ausente. | Cuando el contenido necesite instancias persistentes privadas. |
| Pets/mounts | Ausente. | Después de followers y reglas de party/NPC. |
| Duelo/PvP | Ausente. | Solo con consentimiento, seguridad, muerte/recompensa y moderación diseñados. |
| Facciones/reputación/idiomas | Ausente. | Como contenido data-driven cuando exista una necesidad narrativa. |
| Remort/rebirth | Ausente. | No prioritario mientras v3 aún necesita balance y migraciones. |
| OLC in-game | Ausente. | Como staging/validación/publicación, nunca mutación libre del runtime. |

## Plan por fases y aceptación

### Fase 1 — comunicación y party

- party chat, reply, revocación/kick/transfer/disband;
- canal newbie/general/staff opt-in y moderado;
- paridad browser, texto Telnet y GMCP;
- pruebas de ignore/block/mute, límites, desconexión y cero fuga entre parties.

### Fase 2 — staff por capacidades

- perfiles y capacidades separados de progresión;
- cola de reportes y razones obligatorias;
- inspect/where/goto/transfer/restore/announce/audit acotados;
- autorización negativa, jerarquía, persistencia y auditoría en SQLite/Postgres;
- ninguna autoridad obtenible mediante XP, clase, talento o WorldPack.

### Fase 3 — objetos repetibles e inventario

- migración prototype/instance;
- slots/peso, stacks y contenedores acotados;
- give e intercambio confirmado;
- snapshot, rollback, migración y proyección opaca en ambos transportes.

### Fase 4 — propiedad, rareza y botín aleatorio

- elegibilidad y distribución de party;
- rareza/item level/afijos con presupuestos;
- tablas enteras validadas y tirada transaccional restart-safe;
- probabilidades visibles, pruebas estadísticas deterministas y límites de
  economía;
- cofres solo como recompensa de juego en el baseline.

### Fase 5 — comunidad y contenido

- emotes, mail/boards y guilds según necesidades observadas;
- crafting/auction/housing solo con sinks, cuotas y moderación;
- OLC únicamente sobre el pipeline de artefactos revisados.

Cada fase debe preservar el single writer, una sola persistencia seleccionada,
rollback exacto, límites de frame/cola, percepción antes de protocolo, IDs
opacos y equivalencia funcional entre browser y Telnet.
