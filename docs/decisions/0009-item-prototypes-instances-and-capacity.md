# ADR 0009: prototipos, instancias y capacidad de inventario

- Estado: aceptado; fase 3A implementada. Fase 3B implementada por
  [ADR 0010](0010-derived-stacks-and-bounded-containers.md) y fase 3C por
  [ADR 0011](0011-atomic-give-and-item-transfer-policies.md); fases 3D–4 pendientes.
- Fecha: 2026-07-16.
- Alcance: identidad de objetos, autoría, snapshot, capacidad y presentación.

## Contexto

En WorldPack v3 cada entrada `items[]` cumple a la vez tres funciones: define
el nombre y las reglas de un objeto, identifica una unidad única y participa en
su colocación inicial. El runtime conserva esa misma cadena en salas,
inventarios, equipo, botín de NPC, eventos y snapshot.

El modelo es seguro para el botín fijo actual, pero impide crear dos espadas a
partir de la misma definición sin copiarla y asignarle otro significado
autorial. Tampoco permite pilas, procedencia, comercio confirmado o generación
de botín repetible sin inventar identidad durante una mutación.

Rareza y capacidad son además conceptos diferentes. La rareza describe una
categoría de presentación y presupuesto de poder; los slots y el peso limitan
lo que porta un personaje. Ninguno concede autoridad de cuenta.

## Decisión

### Autoría versionada

WorldPack sube a v4 y AreaPack a v3. El formato nativo separa:

- `itemPrototypes`: definición reusable, reglas, peso, coste de slot, rareza,
  nivel, política de pila y política de transferencia;
- `initialItemInstances`: unidad inicial con ID durable, prototipo y una única
  ubicación `room`, `npc_spawn` o `staged`.

Las reglas dejan de vivir en un mapa paralelo y la ubicación deja de repartirse
entre `room.items` y `lootItemIds`. Un prototipo no posee ubicación. Una
instancia posee exactamente una.

Los lectores aceptan v3/v2 solo en la frontera y los normalizan antes de
validar o componer. Cada objeto legado produce un prototipo y una instancia sin
deduplicar por nombre o contenido. Ambos pueden conservar la misma cadena
histórica porque pertenecen a espacios tipados distintos; la identidad de
instancia publicada no cambia. Un VNUM histórico produce un prototipo y cada
reset inequívoco produce una instancia distinta.

### Identidad runtime

`ItemInstanceId` es un newtype transparente sobre una cadena durable. No es un
UUID aleatorio ni sale a ningún cliente. Los IDs nuevos se derivan de un
contador monotónico persistido y acotado; checkpoint y rollback incluyen el
contador para que un fallo no consuma identidad silenciosamente.

El mundo conserva dos estructuras con responsabilidades distintas:

```text
prototype_id -> ItemPrototype
instance_id  -> ItemInstance { prototype_id, location }
```

Nombre, descripción, aliases, reglas, peso, rareza y nivel se resuelven siempre
desde el prototipo. Equipo, botín, eventos y acciones siempre referencian una
instancia. No existe una tercera identidad de “objeto de inventario”.

Web, Telnet y GMCP siguen recibiendo únicamente IDs opacos derivados de una sal
criptográficamente aleatoria nueva por sesión. Dos unidades del mismo prototipo
tienen IDs opacos distintos; una consulta textual ambigua falla en vez de
elegir una arbitrariamente. Telnet incluye la referencia opaca en escena,
inventario y equipo y la acepta como selector exacto en comandos.
`prototype_id`, el ID durable y el spawn permanecen internos. La ubicación
inicial authored no es una procedencia durable de runtime: ese campo sigue
pendiente para la fase 4.

La autoría separa también identidad visual y percepción. Un registro
`entityMedia` de tipo `item` referencia el prototipo reusable y se proyecta
sobre cada instancia visible de ese prototipo. Una entrada
`roomPerception.hiddenItems` referencia una instancia concreta: ocultarla no
oculta automáticamente otras unidades con el mismo prototipo.

### Snapshot y persistencia

`WorldRuntimeSnapshot` sube a versión 2. Un DTO legado explícito convierte v1:

- cada clave de `item_locations` se convierte en `ItemInstanceId`;
- el prototipo legado conserva esa cadena en su espacio tipado;
- botín y equipo se validan contra las instancias resultantes;
- referencias huérfanas, ubicaciones incompatibles o contadores inválidos
  bloquean el arranque.

Al restaurar un snapshot escrito con un WorldPack anterior, el runtime parte de
las instancias iniciales del WorldPack actual y superpone las instancias y
ubicaciones durables del snapshot. Así incorpora una instancia inicial nueva
sin resucitar una instancia creada en runtime que no esté en el snapshot, y
rechaza que un ID inicial cambie de prototipo.

Antes de abrir listeners, el servidor enumera de forma acotada los propietarios
de instancias portadas o equipadas y carga sus agregados durables por lotes. La
barrera comprueba que cada propietario exista, que equipo y ubicación coincidan
y que slots/peso respeten la capacidad, también para personajes desconectados.
Los agregados solo viven durante esta validación y no quedan conectados ni
retenidos en el estado activo.

La subfase 3A mantiene el snapshot acotado del mundo como única autoridad
durable para instancias. SQLite y PostgreSQL ya lo confirman junto con
personajes y eventos en la misma transacción. No se añade una tabla relacional
espejo ni un esquema v11 hasta que una consulta operativa o una escala medida
lo justifique; dos copias mutables serían menos seguras que el snapshot único.

El tamaño y el número de instancias permanecen acotados: el WorldPack admite
como máximo 1.024 prototipos y 8.192 instancias iniciales; el runtime, 65.536
instancias; el snapshot de mundo, 1 MiB; y la carga agregada de fichas de sus
propietarios durante el arranque, 1 MiB. Cada ficha sigue limitada a 64 KiB.
Si fases posteriores necesitan paginación, descarga por área o consultas
independientes, una
migración futura podrá mover la autoridad a tablas relacionales en una barrera
versionada, nunca mediante doble escritura indefinida.

Esta es la decisión histórica de 3A. ADR 0010 llevó el snapshot a v3 para el
grafo de contenedores y ADR 0011 a v4 para el `binding` obligatorio por
instancia. SQLite y PostgreSQL continúan en esquema v10 y el snapshot sigue
siendo la única autoridad de objetos; no se introdujo tabla espejo ni doble
escritura.

### Capacidad y rareza

Cada personaje tiene `max_slots` y `max_weight_grams`. Cada prototipo declara
`slot_cost` y `weight_grams`. El peso suma instancias portadas y equipadas con
aritmética comprobada; los slots cuentan las portadas. Equipar libera los slots
de esa instancia y desequipar debe reservarlos antes de mutar.

Take, give de script, reemplazo de equipo y unequip validan la operación
completa antes de cambiar estado. El exceso devuelve un error explícito y no
altera ubicación, equipo, revisión ni contador.

La rareza canónica es cerrada: `common`, `uncommon`, `rare`, `epic`,
`legendary`. Se muestra también como texto en todas las superficies; el color
puede acompañar, nunca sustituir la etiqueta. `item_level` es otro eje y no
salta requisitos, límites o reglas económicas.

En 3A toda instancia era individual y transferible: solo se aceptaban
`stackPolicy: unique` y `transferPolicy: transferable`. Los valores
`character_bound` y `non_transferable` se conservan en el vocabulario cerrado,
pero el validador de aquella fase los rechazaba como reservados. ADR 0011
activa las tres políticas solo en WorldPack v5/AreaPack v4 y conserva
WorldPack v4/AreaPack v3 y entradas anteriores como `transferable`; no
reinterpreta el contrato histórico. Una entrada legacy que intente declarar
otra política se rechaza en vez de ignorarla o normalizarla silenciosamente.
Una pila solo puede agrupar unidades
fungibles con prototipo y metadatos idénticos; no se introduce una cantidad que
oculte instancias heterogéneas.

## Atomicidad e invariantes

- Una instancia referencia exactamente un prototipo existente.
- Una instancia ocupa exactamente una ubicación válida, incluido un sink
  terminal cuando se consume, vende o elimina.
- Una unidad no puede estar a la vez en sala, inventario, equipo o NPC.
- La creación, asignación, evento y snapshot de una unidad se confirman juntos.
- Checkpoint/rollback restaura instancias, equipo, contador, RNG y revisiones.
- Los eventos históricos conservan `itemId` como cadena serializada, pero su
  significado interno es siempre ID de instancia.
- La normalización antigua es determinista e idempotente; reiniciar no duplica
  unidades.
- La rareza no modifica autorización y la capacidad nunca se calcula en el
  cliente.

## Secuencia posterior

1. Fase 3A: split prototipo/instancia, snapshot v2, capacidad y rareza textual.
2. Fase 3B, implementada por ADR 0010: pilas fungibles derivadas y contenedores
   con profundidad, peso y capacidad máximos.
3. Fase 3C, implementada por ADR 0011: `give` atómico, propietario efectivo,
   vinculación durable y políticas `character_bound`/`non_transferable`.
4. Fase 3D: intercambio con escrow, doble aceptación e invalidación ante
   cualquier cambio.
5. Fase 4: elegibilidad/reparto de grupo, procedencia, tablas enteras y RNG
   persistido.

No habrá botín aleatorio antes de tener identidad durable, propiedad,
capacidad, derechos de loot y creación/asignación transaccional. Los cofres
obtenidos jugando requerirán probabilidades visibles. Cajas, llaves o tiradas
pagadas quedan fuera del baseline.

## Consecuencias

El formato cambia de versión porque el significado de `items[]` no puede
alterarse silenciosamente. La normalización conserva partidas e IDs históricos,
pero autores y herramientas nuevas deben usar los dos catálogos.

La migración transversal de fase 3A está integrada en WorldPack, core,
compositor, snapshot, Web y Telnet/GMCP. Tiamud obtiene así una base única para
objetos repetibles sin introducir todavía tablas de botín, comercio o una
segunda autoridad persistente prematura. ADR 0010 añade pilas derivadas y
contenedores acotados sobre esa base sin reinterpretar esta decisión. ADR 0011
añade la entrega simple y las políticas ejecutables sobre el subárbol completo.
El intercambio confirmado, derechos/reparto y botín aleatorio permanecen en
las fases posteriores.
