# Autoría, validación y publicación de mundos

Esta guía describe el contrato vigente de autoría de Tiamud: **WorldPack v9**
como artefacto canónico de ejecución, **AreaPack v8** como módulo de autoría y
`WorldProjectManifest` v2 como composición fuera de línea. La autoridad exacta
son los tipos y validadores de `crates/world-pack`; el ejemplo revisado es
[`worlds/lantern-square.world.json`](../worlds/lantern-square.world.json).

La [especificación v0.3](spec-v0.3.md) resume el producto y el
[centro de autoría](authoring/README.md) amplía personajes, medios, percepción,
variantes, scripts y pruebas de contenido.

## Frontera de ejecución

El servidor valida y carga un único `WorldPackDocument` v9 completo antes de
abrir sus listeners. `TIAMUD_WORLD_PACK` permite seleccionar un artefacto
externo revisado; si no se define, se usa Lantern Square embebido.

Un WorldPack compuesto no contiene áreas runtime. `AreaPack` es propiedad
editorial modular: `tiamud-world compose` resuelve sus dependencias y produce
un único artefacto plano. Por tanto:

- no hay composición, descarga ni resolución de dependencias durante el juego;
- no existen salidas “entre paquetes” runtime: todo destino debe existir en el
  WorldPack final;
- cambiar el mundo requiere seleccionar el artefacto y reiniciar;
- la carga/reemplazo caliente y la expulsión LRU del ADR 0005 siguen pendientes;
- los IDs publicados son identidad durable, no títulos; y
- perder imagen, audio o color nunca elimina prosa o acciones jugables.

```mermaid
flowchart LR
    accTitle: Flujo vigente de autoría modular de Tiamud
    accDescr: Las áreas se componen y validan fuera de línea para producir un solo WorldPack canónico que el servidor carga al arrancar. No existe carga caliente.

    Project[WorldProjectManifest v2] --> Compose[tiamud-world compose]
    Areas[AreaPack v8<br/>1..=256 módulos] --> Compose
    Compose --> Validate[Validación global<br/>referencias · límites · Lua]
    Validate --> Artifact[WorldPack v9 canónico]
    Artifact --> Server[Servidor único<br/>carga al arrancar]
    Hot[Catálogo caliente/LRU] -. futuro .-> Server
```

## Versiones y compatibilidad

| Entrada | Tratamiento |
| --- | --- |
| WorldPack v9 | Formato canónico nativo. |
| WorldPack v8/v7/v6/v5/v4/v3/v2 | Se normaliza de forma determinista a v9 antes de validar, sin inventar emotes; v2-v7 tampoco inventan comunicación, v2-v6 tampoco inventan sellos ni afijos y v2-v5 tampoco inventan tablas ni fuentes de botín. |
| AreaPack v8 | Formato modular nativo. |
| AreaPack v7/v6/v5/v4/v3/v2/v1 | Se normaliza a v8 antes de componer, sin inventar emotes; v1-v6 tampoco inventan comunicación, v1-v5 tampoco inventan sellos ni afijos y v1-v4 tampoco inventan tablas ni fuentes de botín. |
| Envy/Merc | Solo pasa por `envy-import`; los programas históricos nunca se ejecutan. |

La compatibilidad antigua no permite escribir vocabulario legado en un
documento nativo. WorldPack v9 y AreaPack v8 rechazan `items`, `room.items`,
`itemRules` y `lootItemIds`: usan `itemPrototypes` e
`initialItemInstances`.

`lootTables` y `lootSources` nacen en WorldPack v6 y AreaPack v5. Declararlos,
aunque estén vacíos, en WorldPack v2-v5 o AreaPack v1-v4 se rechaza como mezcla
de esquemas. La conversión de esos formatos crea colecciones vacías: ni el
normalizador ni `envy-import` deducen probabilidades a partir de resets o listas
históricas ambiguas.

Las recompensas selladas, `affixes`, `affixPools` y `affixPoolId` nacen en
WorldPack v7 y AreaPack v6. Declararlos, aunque estén vacíos, en formatos
anteriores se rechaza como mezcla de esquemas. La normalización conserva el
botín normal de v6/v5, pero no inventa sellos, probabilidades secundarias ni
afijos.

La comunicación authored nace en WorldPack v8 y AreaPack v7. Un campo
`ability.communication`, aunque sea una forma vacía o parcial, se rechaza en
WorldPack v2-v7 y AreaPack v1-v6 en vez de interpretarse como una habilidad
ordinaria. La normalización deja `communication` ausente y `envy-import` no lo
deduce de prosa, programas o flags históricos.

Los emotes authored nacen en WorldPack v9 y AreaPack v8. `socialEmotes`,
aunque sea un array vacío, se rechaza en WorldPack v2-v8 y AreaPack v1-v7.
La normalización produce una colección vacía y `envy-import` nunca convierte
sociales, programas o prosa histórica en plantillas ejecutables.

WorldPack v4 y AreaPack v3 reciben por normalización `maxStackUnits: 1`, sin
contenedor y con `transferPolicy: transferable`. En esas versiones la política
debe ser exactamente `transferable`; `character_bound` y `non_transferable`
no se reinterpretan como si el formato antiguo ya conociera la vinculación de
3C. Si una entrada antigua intenta declarar campos de 3B/3C, se rechaza en vez
de ignorarlos: debe declarar la versión nativa correspondiente. WorldPack
v3/v2, AreaPack v2/v1 y Envy/Merc también normalizan siempre a objetos
transferibles y nunca infieren un vínculo desde flags históricos.

Los objetos v3/v2 antiguos se convierten uno a uno, sin deduplicar por nombre o
contenido. El prototipo y la instancia pueden conservar la misma cadena
histórica porque pertenecen a espacios tipados diferentes. Un VNUM importado
produce un prototipo; cada reset inequívoco produce su propia instancia.

## Proyecto modular v2

El manifiesto contiene la identidad global, sala inicial, semilla y defaults de
personaje. Las rutas de área son relativas, no escapan del directorio del
proyecto y son únicas.

```text
WorldProjectManifest v2
├── schemaVersion! = 2
├── id!
├── name!
├── startRoomId!
├── rngSeed?
├── defaultLineageId?
├── defaultBackgroundId?
├── defaultBaseClassId?
└── areas[]!             1..=256
    ├── id!
    └── path!
```

Ejemplo:

```json
{
  "schemaVersion": 2,
  "id": "ejemplo-modular",
  "name": "Mundo modular de ejemplo",
  "startRoomId": "plaza:centro",
  "rngSeed": 42,
  "defaultLineageId": "ejemplo:mortal",
  "defaultBaseClassId": "ejemplo:viajero",
  "areas": [
    { "id": "plaza", "path": "areas/plaza.area.json" },
    { "id": "puerta", "path": "areas/puerta.area.json" }
  ]
}
```

## AreaPack v8

Cada área reutiliza los mismos registros del WorldPack; no crea una segunda
representación de salas, objetos o NPC. Solo añade `id` y `dependsOn` como
frontera de propiedad.

```text
AreaPack v8
├── schemaVersion! = 8
├── id!
├── dependsOn[]?
├── itemPrototypes[]?
├── initialItemInstances[]?
├── rooms[]?
├── npcPrototypes[]?
├── npcSpawns[]?
├── lootTables[]?
├── lootSources[]?
├── affixes[]?
├── affixPools[]?
├── levelThresholds[]?
├── lineages[]?
├── backgrounds[]?
├── baseClasses[]?
├── advancedClasses[]?
├── talentGraphs[]?
├── abilities[]?
├── socialEmotes[]?
├── npcAbilitySets[]?
├── roomMedia[]?
├── entityMedia[]?
├── roomPerception[]?
├── sceneVariants[]?
├── scripts[]?
└── contentTests[]?
```

Una dependencia puede ser cíclica: dos áreas con salidas de ida y vuelta son un
caso normal. Toda referencia estructurada a una entidad de otra área debe
declarar su `id` en `dependsOn`. Esto incluye:

- salidas a salas ajenas;
- instancia→prototipo y ubicación de instancia en sala, spawn o contenedor ajenos;
- spawn→prototipo NPC y spawn→sala;
- tabla de botín→prototipo y fuente de botín→spawn/tabla ajenos;
- concesiones/requisitos de progresión y habilidades;
- multimedia, percepción, variantes, scripts y pruebas.

El compositor no inspecciona texto Lua para inventar dependencias. Las
referencias opacas dentro de `source` son responsabilidad del autor.

### Ejemplo de dos áreas conectadas

`areas/plaza.area.json`:

```json
{
  "schemaVersion": 8,
  "id": "plaza",
  "dependsOn": ["puerta"],
  "itemPrototypes": [
    {
      "id": "plaza:farol",
      "name": "farol azul",
      "description": "Un farol de cristal azul.",
      "aliases": ["farol", "lámpara"],
      "equipmentSlot": "trinket",
      "weightGrams": 1200,
      "slotCost": 2,
      "rarity": "common",
      "itemLevel": 1,
      "stackPolicy": "unique",
      "maxStackUnits": 1,
      "transferPolicy": "transferable"
    }
  ],
  "initialItemInstances": [
    {
      "id": "plaza:farol-1",
      "prototypeId": "plaza:farol",
      "location": { "kind": "room", "roomId": "plaza:centro" }
    }
  ],
  "rooms": [
    {
      "id": "plaza:centro",
      "title": "La plaza",
      "description": "Un camino sale hacia el norte.",
      "exits": [
        {
          "direction": "north",
          "target": { "kind": "room", "room_id": "puerta:entrada" }
        }
      ]
    }
  ]
}
```

`areas/puerta.area.json`:

```json
{
  "schemaVersion": 5,
  "id": "puerta",
  "dependsOn": ["plaza"],
  "rooms": [
    {
      "id": "puerta:entrada",
      "title": "La puerta norte",
      "description": "La plaza queda al sur.",
      "exits": [
        {
          "direction": "south",
          "target": { "kind": "room", "room_id": "plaza:centro" }
        }
      ]
    }
  ],
  "npcPrototypes": [
    {
      "id": "puerta:lobo",
      "name": "lobo de lluvia",
      "description": "Vigila el camino bajo la lluvia.",
      "level": 1,
      "maxHealth": 18,
      "strength": 3,
      "defence": 1,
      "experienceReward": 60,
      "respawnTicks": 8
    }
  ],
  "npcSpawns": [
    {
      "id": "puerta:lobo-1",
      "prototypeId": "puerta:lobo",
      "roomId": "puerta:entrada"
    }
  ],
  "itemPrototypes": [
    {
      "id": "puerta:colmillo",
      "name": "colmillo plateado",
      "description": "Un pequeño colmillo engarzado en plata.",
      "aliases": ["colmillo"],
      "weightGrams": 100,
      "slotCost": 1,
      "rarity": "uncommon",
      "itemLevel": 1,
      "stackPolicy": "unique",
      "maxStackUnits": 1,
      "transferPolicy": "transferable"
    }
  ],
  "initialItemInstances": [
    {
      "id": "puerta:colmillo-1",
      "prototypeId": "puerta:colmillo",
      "location": {
        "kind": "npc_spawn",
        "npcSpawnId": "puerta:lobo-1"
      }
    }
  ]
}
```

El botín no vive en `npcPrototypes`. La ubicación `npc_spawn` de una instancia
es botín fijo authored, asignado una sola vez y validado. El botín repetible se
declara por separado mediante `lootTables` y `lootSources`, como se detalla más
abajo.

## WorldPack v9

El artefacto plano conserva identidad, catálogos de objeto, salas y gameplay.
Las extensiones de multimedia/percepción/scripts se aplanan en el documento
por `WorldPackDocument` aunque la estructura Rust separa el núcleo del mundo.

```text
WorldPack v9
├── schemaVersion! = 9
├── id!
├── name!
├── startRoomId!
├── source?
├── itemPrototypes[]?
├── initialItemInstances[]?
├── rooms[]!
├── gameplay?
│   ├── rngSeed?
│   ├── npcPrototypes[]?
│   ├── npcSpawns[]?
│   ├── characterProgression?
│   ├── abilities[]?
│   ├── socialEmotes[]?
│   ├── npcAbilitySets[]?
│   ├── lootTables[]?
│   ├── lootSources[]?
│   ├── affixes[]?
│   └── affixPools[]?
├── roomMedia[]?
├── entityMedia[]?
├── roomPerception[]?
├── sceneVariants[]?
├── scripts[]?
└── contentTests[]?
```

Un mundo mínimo sin objetos:

```json
{
  "schemaVersion": 9,
  "id": "ejemplo",
  "name": "Mundo de ejemplo",
  "startRoomId": "ejemplo:entrada",
  "rooms": [
    {
      "id": "ejemplo:entrada",
      "title": "La entrada",
      "description": "Un arco de piedra conduce al camino."
    }
  ]
}
```

Cada sala y variante exige un `title` de 1..=128 bytes UTF-8 seguros, sin
caracteres de control, y una `description` de 1..=8.192 bytes UTF-8 seguros. Los saltos de
línea y tabulaciones revisados son válidos en la descripción; una cadena vacía,
solo whitespace, una secuencia de control insegura o un exceso de bytes se
rechazan. Los límites cuentan bytes, no caracteres Unicode.

## Habilidades de comunicación y telepatía

WorldPack v9/AreaPack v8 conserva la comunicación introducida por WorldPack
v8/AreaPack v7 y reutiliza `AbilityDefinition`: la telepatía no es un
canal global ni un `tell` renombrado. El personaje debe conocer la habilidad,
tener su recurso disponible y pagar un coste positivo; la habilidad tiene un
cooldown authored positivo y conserva el cooldown global de habilidades.

```json
{
  "id": "tiamud:mind-whisper",
  "name": "Mind Whisper",
  "description": "Envía un pensamiento privado a un aliado conectado.",
  "category": "utility",
  "resourceCosts": { "stamina": 2 },
  "cooldownTicks": 2,
  "targeting": "single_ally",
  "communication": {
    "kind": "telepathy",
    "scope": "direct_online",
    "blockedRoomIds": ["demo:archive"]
  }
}
```

El vocabulario es cerrado: solo `kind: telepathy` y
`scope: direct_online`. La habilidad debe ser `spell` o `utility`, usar
`single_ally` y no puede combinar comunicación con daño, curación ni estados.
Un mundo admite como máximo 32 habilidades de comunicación; cada una enumera
hasta 64 salas bloqueadas, únicas y existentes. Una referencia a una sala de
otra AreaPack exige la correspondiente dependencia `dependsOn`. Ningún
`npcAbilitySet` puede incluirla.

Lantern Square concede `tiamud:mind-whisper` a `tiamud:wanderer`, la clase
predeterminada, cuyo pool de stamina permite pagarla desde nivel 1. El Archivo
demuestra un bloqueo authored. El mensaje y el destinatario son input runtime:
no se escriben en el WorldPack.

## Emotes sociales authored

WorldPack v9 declara `gameplay.socialEmotes`; AreaPack v8 usa `socialEmotes`
en su raíz. Hay como máximo 32 por mundo. Cada definición contiene un `id`
namespaced de hasta 64 bytes, un `command` ASCII único que cumple
`[a-z][a-z0-9-]*` (hasta 32 bytes), un nombre seguro de hasta 64 bytes,
`audience: room` y `targeting: none|optional_character|required_character`.

```json
{
  "id": "tiamud:smile",
  "command": "smile",
  "name": "Sonreír",
  "audience": "room",
  "targeting": "optional_character",
  "templates": {
    "untargeted": {
      "actor": "Sonríes.",
      "others": "{actor} sonríe."
    },
    "targeted": {
      "actor": "Sonríes a {target}.",
      "target": "{actor} te sonríe.",
      "others": "{actor} sonríe a {target}."
    }
  }
}
```

`none` exige solo `untargeted`, `required_character` solo `targeted` y
`optional_character` ambos. Sin objetivo, `actor` no admite placeholder y
`others` exige `{actor}` exactamente una vez. Con objetivo, `actor` exige solo
`{target}`, `target` solo `{actor}` y `others` uno de cada. Se rechazan llaves
o placeholders adicionales, controles, ESC, CR/LF, `<`, `>` y `&`; el
resultado de sustituir nombres de 64 bytes debe caber en 512 bytes. Las
plantillas no conceden autoridad ni visibilidad: el runtime calcula la
audiencia de la sala.

## Prototipos e instancias de objeto

`itemPrototypes` define contenido reutilizable. `initialItemInstances` define
unidades iniciales y su única ubicación.

```text
ItemPrototype
├── id!
├── name!
├── description!
├── aliases[]?
├── equipmentSlot?       weapon | body | trinket
├── attackBonus?
├── healAmount?
├── consumable?
├── effect?
├── weightGrams!         0..=1_000_000
├── slotCost!            1..=100
├── rarity!              common | uncommon | rare | epic | legendary
├── itemLevel!           1..=1_000
├── stackPolicy!         unique | fungible
├── maxStackUnits!       unique = 1; fungible = 2..=1.000
├── container?
│   ├── maxSlots!        1..=1.000
│   └── maxWeightGrams!  0..=1.000.000
└── transferPolicy!      transferable | character_bound | non_transferable

InitialItemInstance
├── id!                  ID durable interno
├── prototypeId!         prototipo existente
└── location!
    ├── { kind: room, roomId }
    ├── { kind: npc_spawn, npcSpawnId }
    ├── { kind: container, containerInstanceId }
    └── { kind: staged }
```

Invariantes:

- cada instancia referencia exactamente un prototipo;
- cada ID de instancia authored ocupa 1..=128 bytes y usa la gramática cerrada
  de identificadores gameplay; los IDs runtime son `runtime:<u64-canónico>`;
- cada instancia tiene una sola ubicación;
- un prototipo no tiene ubicación;
- dos instancias pueden compartir prototipo, nunca identidad;
- una instancia `npc_spawn` referencia un spawn existente;
- una instancia `container` referencia otra instancia cuyo prototipo define
  contenedor;
- el grafo de contención no admite autorreferencia, ciclos ni más de cuatro
  aristas;
- las instancias nuevas de runtime usan una secuencia durable incluida en
  checkpoint y rollback;
- el servidor resuelve nombre, descripción, peso, rareza y reglas desde el
  prototipo;
- `entityMedia(kind=item)` referencia un prototipo y puede presentar cada una
  de sus instancias visibles;
- `roomPerception.hiddenItems` referencia una instancia concreta;
- Web, Telnet y GMCP reciben solo IDs opacos derivados de una sal
  criptográficamente aleatoria nueva por sesión;
- Telnet muestra esas referencias en escena, inventario y equipo y las acepta
  como selector exacto; y
- una referencia opaca selecciona la unidad o grupo exactos; una consulta
  textual ambigua falla en vez de escoger una unidad arbitraria.

La capacidad de personaje no se autoriza en el WorldPack ni se calcula en el
cliente. Los slots del personaje cuentan raíces portadas, agrupadas por la
pila derivada de prototipo y vínculo; el peso incluye raíces portadas/equipadas
y todos sus descendientes. Los slots de un contenedor cuentan hijos directos
agrupados por esa misma clave y su peso usado
cuenta todo el subárbol, sin incluir la carcasa del propio contenedor. Equipar,
desequipar, reemplazar, tomar, soltar, introducir y extraer prevalidan el
resultado completo.

```mermaid
flowchart TD
    Character[Personaje] -->|slots: solo raíz| Bag[Bolsa unique]
    Bag -->|slots directos| TonicA[Tónica]
    Bag --> TonicB[Tónica]
    Bag --> Box[Caja unique]
    Box --> Map[Mapa]
    TonicA -. pila fungible derivada x2 .- TonicB
    Character -. peso: carcasa y descendientes .-> Map
```

Las pilas nunca son entidades durables. Se derivan por
ubicación/prototipo/vínculo,
ordenan sus instancias por ID y se dividen por `maxStackUnits`:

```text
stackSlots = ceil(quantity / maxStackUnits) * slotCost
totalWeightGrams = quantity * weightGrams
```

La cantidad predeterminada es uno y una mutación acepta como máximo 256
unidades. `put` opera sobre raíces portadas; `get` solo sobre hijos directos;
`contents` pagina veinte grupos. Mover una instancia contenedor conserva las
ubicaciones relativas de todo su subárbol.

### Propiedad efectiva y transferencia

WorldPack v9 y AreaPack v8 aceptan las tres políticas. WorldPack v5 y AreaPack
v4 también las aceptaban en su frontera histórica. La vinculación no se escribe
en autoría: pertenece a cada instancia runtime y se crea, valida y persiste
cuando el personaje adquiere el objeto.

- `transferable` puede cambiar de propietario o volver al mundo si todas las
  instancias de su subárbol también son transferibles;
- `character_bound` se vincula al `CharacterId` de la primera adquisición, no
  se desliga ni se reasigna y conserva el vínculo al consumirse; y
- `non_transferable` no guarda vínculo, pero una vez dentro del grafo de un
  personaje no puede soltarse ni pasar a otro; su consumo autorizado sí puede
  llevarlo al estado terminal `Consumed`.

El propietario efectivo se obtiene siguiendo hasta cuatro padres `Contained`
hasta una raíz `Carried` o `Equipped`. Por eso una bolsa `transferable` que
contenga un objeto ligado o no transferible puede reorganizarse dentro del
inventario de su dueño, pero no soltarse ni entregarse. `take`, `drop`,
`put`, `get`, equipo, consumo y la concesión autorizada de Lua prevalidan
ubicación, vínculo, grafo y capacidad sobre una copia prospectiva completa.

La entrega directa usa la misma acción en Web y Telnet:

```text
give [cantidad] <objeto> to <personaje>
```

Solo mueve raíces `Carried` y conserva cada subárbol. El receptor debe ser
distinto, estar conectado y en la misma sala; se resuelve por selector opaco o
nombre exacto no ambiguo. La entrega exige un subárbol completamente
`transferable`, capacidad válida en ambos personajes y ausencia de ignore del
receptor o block bilateral. Todo el lote de 1..=256 raíces se confirma con una
sola mutación y commit; `canGive` y las acciones proyectadas solo ayudan a la
interfaz, nunca sustituyen esta autorización del servidor.

La fase 3D implementa intercambio confirmado como estado runtime, no como
autoría del WorldPack. Solo reserva raíces `Carried` cuyo subárbol completo es
`transferable`; no añade campos de trade, escrow ni propietario a
`ItemPrototype` o `InitialItemInstance`. Siguen ausentes entrega offline/remota
y los repartos need/greed, master-loot o por contribución.

## Botín fijo y repetible

Una aparición de NPC puede combinar dos fuentes que convergen en la misma
resolución autoritativa:

- cada raíz inicial con ubicación `npc_spawn` es botín fijo authored y solo se
  entrega una vez; y
- una `lootSource` enlaza esa aparición con una `lootTable`, que se resuelve de
  nuevo en cada derrota.

La tabla procesa primero `guaranteed` y después `pools`, ambos ordenados por ID
canónico. Cada pool realiza `rolls` selecciones con reemplazo. Una entrada
`item` usa un peso entero positivo y una cantidad uniforme inclusiva entre
`minQuantity` y `maxQuantity`; una entrada `nothing` consume la tirada sin crear
una instancia y no admite prototipo ni cantidades. El runtime usa rejection
sampling, no porcentajes flotantes ni módulo sesgado.

AreaPack v8 declara ambas colecciones en su nivel superior; WorldPack v9 las
sitúa dentro de `gameplay`. El botín normal se introdujo en AreaPack v5 y
WorldPack v6. El fragmento siguiente muestra su forma común actual:

```json
{
  "lootTables": [
    {
      "id": "tiamud:gate-stalker-drops",
      "guaranteed": [
        {
          "id": "tiamud:drop:road-token",
          "itemPrototypeId": "tiamud:road-token",
          "quantity": 1
        },
        {
          "id": "tiamud:drop:storm-seal",
          "itemPrototypeId": "tiamud:storm-seal",
          "quantity": 1
        }
      ],
      "pools": [
        {
          "id": "tiamud:drop:trophy",
          "rolls": 1,
          "entries": [
            {
              "kind": "item",
              "id": "tiamud:drop:wolf-clasp",
              "weight": 6000,
              "itemPrototypeId": "tiamud:wolf-clasp",
              "minQuantity": 1,
              "maxQuantity": 1,
              "affixPoolId": "tiamud:storm-talon-affixes"
            },
            {
              "kind": "nothing",
              "id": "tiamud:drop:nothing",
              "weight": 4000
            }
          ]
        }
      ]
    }
  ],
  "lootSources": [
    {
      "kind": "npc_spawn",
      "id": "tiamud:gate-stalker-loot",
      "npcSpawnId": "tiamud:gate-stalker-1",
      "lootTableId": "tiamud:gate-stalker-drops",
      "rights": "party_round_robin"
    }
  ]
}
```

En la demostración, cada derrota produce un `tiamud:road-token` y un
`tiamud:storm-seal` garantizados y realiza una tirada: `tiamud:wolf-clasp`
tiene peso 6000 y `nothing` 4000, es decir, 60 % frente a 40 %. Además, la instancia inicial
`tiamud:stormglass-talon` permanece como botín fijo de la primera derrota. La
política `party_round_robin` reparte cada raíz sucesiva por una cohorte de hasta
seis personajes; `credited_character` reservaría todas al iniciador. La
selección inicial no es discrecional del líder.

Reglas de referencias y capacidad:

- IDs de tabla, fuente, entrada garantizada, pool y entrada ponderada son
  canónicos, únicos en su ámbito y ocupan 1..=128 bytes;
- cada prototipo, aparición de NPC y tabla referenciados debe existir;
- una aparición admite como máximo una fuente y toda tabla debe estar usada;
- una tabla admite hasta 8 entradas garantizadas y 8 pools; cada pool, 1..=8
  tiradas y 1..=64 entradas;
- el WorldPack admite 256 tablas, 1.024 fuentes y 4.096 entradas ponderadas;
- cada peso está en `1..=1.000.000` y cada cantidad en `1..=32`; y
- el peor caso de raíces fijas más unidades repetibles para una aparición no
  puede superar 64 por derrota.

## Sellos ganados y afijos (fase 4B)

WorldPack v9 y AreaPack v8 conservan las recompensas selladas obtenidas jugando
y un único afijo opcional por unidad de botín `unique`, introducidos en v7/v6.
La decisión autoritativa es
el [ADR 0014](decisions/0014-earned-sealed-rewards-and-deterministic-affixes.md).
No son cajas pagadas: no existen compra de tiradas, llaves, pity ni una moneda
premium implícita.

Un prototipo sellado declara `sealedLootTableId` y debe ser `unique`,
`maxStackUnits: 1`, consumible, `character_bound`, sin contenedor, equipo,
ataque, curación ni efecto. No admite instancia inicial. Una tabla normal
asociada a un NPC debe generarlo como recompensa; su tabla de apertura no puede
ser una `lootSource` directa ni producir otro sello. La apertura siempre genera
entre 1 y 16 unidades y su expansión de resultados base más afijos
condicionales admite como máximo 1.024 filas visibles en `reward odds`.

```json
{
  "itemPrototypes": [
    {
      "id": "tiamud:storm-seal",
      "name": "juramento sellado del acechante",
      "description": "Un cordón ennegrecido por la lluvia rodea una esquirla marcada.",
      "consumable": true,
      "weightGrams": 40,
      "slotCost": 1,
      "rarity": "rare",
      "itemLevel": 1,
      "stackPolicy": "unique",
      "maxStackUnits": 1,
      "transferPolicy": "character_bound",
      "sealedLootTableId": "tiamud:storm-oath-rewards"
    }
  ],
  "lootTables": [
    {
      "id": "tiamud:storm-oath-rewards",
      "guaranteed": [
        {
          "id": "tiamud:reward:stormglass-talon",
          "itemPrototypeId": "tiamud:stormglass-talon",
          "quantity": 1,
          "affixPoolId": "tiamud:storm-talon-affixes"
        }
      ]
    }
  ],
  "affixes": [
    {
      "id": "tiamud:affix:storm-bitten",
      "label": "mordido por la tormenta",
      "position": "prefix",
      "attackBonusDelta": 1
    }
  ],
  "affixPools": [
    {
      "id": "tiamud:storm-talon-affixes",
      "entries": [
        {
          "kind": "affix",
          "id": "tiamud:affix-roll:storm-bitten",
          "weight": 6000,
          "affixId": "tiamud:affix:storm-bitten"
        },
        {
          "kind": "nothing",
          "id": "tiamud:affix-roll:plain",
          "weight": 4000
        }
      ]
    }
  ]
}
```

Hay como máximo 512 definiciones de afijo, 256 pools, 64 entradas por pool y
4.096 entradas ponderadas en total. La etiqueta ocupa hasta 64 bytes; cada
delta positivo de ataque o curación llega como máximo a 1.000. Un pool se
referencia solo con cantidad exactamente uno desde un prototipo no sellado,
`unique` y `maxStackUnits: 1`. Un delta de ataque requiere objeto equipable y
uno de curación, consumible; el valor efectivo no puede superar los límites
del runtime. Un objeto recibe como máximo un afijo.

Los pools normales y sellados usan selección ponderada entera, determinista y
sin sesgo. La tirada de afijo normal se consume aunque después no quepa o se
suprima el drop, para que el resultado no dependa de la capacidad observable.
`open <sello>` consume una unidad y `reward odds <sello>` muestra las
probabilidades exactas authored. Consumo, RNG, contador de apertura, creación,
vínculo, capacidad y grupos de resultado —hasta 16 y coherentes con las
instancias creadas— forman un solo commit; cualquier fallo restaura exactamente
el sello, los contadores y el RNG.

`WorldRuntimeSnapshot` v7 conserva por instancia el afijo aplicado y la
procedencia `SealedReward`. Las copias durables incluyen los digests canónicos
de definición, pool y registro, pero clientes Web, Telnet y GMCP solo reciben
la presentación y estadísticas autorizadas: IDs estables, estado RNG,
contadores internos y digests permanecen opacos. Los mismos afijos funcionan
en botín normal y sellado.

El SHA-256 de la forma canónica exacta de cada tabla viaja al dominio. Cada
instancia generada conserva procedencia durable con drop, aparición, tabla,
digest, entrada, pool opcional, índices y tick. Las instancias iniciales se
marcan como authored; las creadas por la API general conservan procedencia
runtime propia. Esta metadata vive en `WorldRuntimeSnapshot` v7 y nunca se
expone a clientes. Cambiar una tabla después no reescribe la procedencia de los
objetos ya generados.

## Composición, validación y digest

```sh
mkdir -p build/worlds
cargo run --locked -p world-pack --bin tiamud-world -- compose \
  worlds/lantern-square.project.json \
  --output build/worlds/lantern-square.composed.world.json

cargo run --locked -p world-pack --bin tiamud-world -- validate \
  build/worlds/lantern-square.composed.world.json
```

La composición:

- rechaza propiedades desconocidas también en registros anidados;
- valida rutas relativas y evita escapes o duplicados;
- exige IDs globales únicos, salvo el par histórico tipado
  prototipo/instancia normalizado con la misma cadena;
- verifica `dependsOn` para referencias estructuradas cruzadas;
- fusiona y valida todas las referencias, límites, condiciones y scripts;
- ordena canónicamente listas no semánticas;
- produce bytes y reportes deterministas; y
- escribe el destino completo de forma atómica sin sobreescribirlo.

Lantern Square se compone desde tres áreas y produce tres salas, diez
prototipos, nueve instancias iniciales, dos tablas de botín, una fuente, un
afijo, un pool, una habilidad de telepatía directa y un emote social authored.
El artefacto de 53.567 bytes es byte a byte idéntico al
WorldPack canónico y su
SHA-256 es:

```text
9a75894a9e2e0c48d20257633f323d52bbe78b9db106de8c497f4784392b5e75
```

La demostración conserva `tiamud:road-token` y `tiamud:wolf-clasp` como
`transferable`: sus IDs publicados no cambian de semántica sin migración. Los
casos 3C son nuevas identidades: `tiamud:rain-writ`, consumible con
`healAmount: 1` y `non_transferable`, comienza en `demo:north-gate`; y
`tiamud:stormglass-talon`, `character_bound`, es botín fijo de
`tiamud:gate-stalker-1`. Esa misma aparición usa reparto round-robin de party
para el `tiamud:road-token` y el `tiamud:storm-seal` garantizados, además de la
tirada 60 % `tiamud:wolf-clasp`/40 % `nothing` de su tabla repetible. El clasp
normal y el talón obtenido al abrir el sello comparten una tirada de afijo:
60 % `storm-bitten` y 40 % sin afijo.
La clase inicial predeterminada concede además `tiamud:mind-whisper`, con coste
de stamina y cooldown positivos; `demo:archive` demuestra su bloqueo authored.

Límites principales:

| Recurso | Límite |
| --- | ---: |
| Manifiesto de proyecto | 1 MiB |
| Número de áreas | 1..=256 |
| Cada fichero de área | 16 MiB |
| Total de fuentes de área | 16 MiB |
| WorldPack compuesto | 16 MiB |
| Ruta relativa | 4096 bytes |
| Título de sala | 1..=128 bytes seguros, sin controles |
| Descripción de sala | 1..=8.192 bytes seguros |
| Prototipos de objeto por WorldPack | 1.024 |
| Instancias iniciales por WorldPack | 8.192 |
| Bytes por ID de instancia | 1..=128 |
| Aliases por prototipo | 32 |
| Peso de un prototipo | 0..=1.000.000 g |
| Coste de slots de un prototipo | 1..=100 |
| `itemLevel` | 1..=1.000 |
| Unidades máximas por pila | 1.000 |
| Slots de un contenedor | 1..=1.000 |
| Peso de contenido por contenedor | 0..=1.000.000 g |
| Profundidad de contención | 4 aristas |
| Unidades movidas por acción | 256 |
| Grupos por página de `contents` | 20 |
| Tablas/fuentes de botín | 256 / 1.024 |
| Garantizados/pools por tabla | 8 / 8 |
| Tiradas/entradas por pool | 8 / 64 |
| Entradas ponderadas globales | 4.096 |
| Peso/cantidad de una entrada | 1..=1.000.000 / 1..=32 |
| Raíces fijas + repetibles por derrota | 64 |

## Flujo editorial

`tiamud-world` comparte el mismo normalizador y validador en todos sus comandos:

```sh
cargo run --locked -p world-pack --bin tiamud-world -- validate mundo.json
cargo run --locked -p world-pack --bin tiamud-world -- preview mundo.json --room <id>
cargo run --locked -p world-pack --bin tiamud-world -- test mundo.json
cargo run --locked -p world-pack --bin tiamud-world -- build mundo.json --output artefacto.json
cargo run --locked -p world-pack --bin tiamud-world -- diff anterior.json nuevo.json
```

Publicación y rollback usan artefactos inmutables por digest. No equivalen a
recarga en caliente: activar un digest prepara qué artefacto usará el proceso al
arrancar o reiniciar.

Antes de publicar:

1. compón las áreas y revisa el `CompositionReport`;
2. valida el artefacto y sus recursos;
3. ejecuta pruebas de contenido y previews textuales;
4. verifica licencia/procedencia de cada input y medio;
5. compara el diff canónico;
6. publica por digest sin sobrescribir; y
7. prueba arranque y rollback con una persistencia desechable.

Los recursos multimedia son presentación inmutable, mismo origen y
direccionada por contenido. Lua se compila y ejecuta con capacidades/cuotas; no
tiene sistema de archivos, red, procesos ni módulos nativos. La IA editorial es
opcional, fuera de ejecución y siempre produce un borrador que requiere
validación y revisión humana.

## Persistencia y evolución del formato

WorldPack define el mundo inicial; `WorldRuntimeSnapshot` v7 define dónde están
las instancias después de jugar, su `binding` obligatorio, los contadores de
identidad y apertura, los afijos aplicados, las reservas de trade, la
procedencia normal o sellada, los encuentros de botín, los derechos sobre suelo
y los cursores round-robin. El decoder elige primero por
`snapshotVersion` y compara esa versión original con la columna SQL. DTO
explícitos v1-v5 convierten de forma estricta: v1-v3 producen
`binding: { kind: "unbound" }` y v4 añade trade vacío. Al llegar a v6, cada
origen v1-v5 añade botín vacío y procedencia
`legacy_snapshot { sourceVersion }` para todas sus instancias. Un v6 exacto
convierte a v7 con `itemAffixes: []` en cada instancia y
`nextRewardOpenId: 1`.
Todas fallan cerradas ante campos cruzados. En particular, inyectar `binding`
en v3, trade en v4, procedencia/derechos v6 en v5 o afijos/contador de apertura
v7 en v6 no adelanta su versión.
También se rechazan referencias, ciclos, profundidad, capacidad, contador,
propietario, vínculo, participante, reserva, cohorte o procedencia inválidos.

Al restaurar un snapshot anterior, las instancias iniciales nuevas del
WorldPack se conservan y el estado durable se superpone; una instancia creada
en runtime que no figure en el snapshot no reaparece. Antes de abrir listeners
se validan por lotes los propietarios de objetos siguiendo padres `Contained`
hasta la raíz y todos los `CharacterId` ligados, incluidos los de objetos
consumidos. Se comprueban sus fichas, equipos, el grafo, los vínculos y la
capacidad, también para personajes desconectados. Los participantes de trade
que no sean ya propietarios se cargan con un presupuesto adicional cerrado;
las identidades que solo aparecen en cohortes o derechos de botín se comprueban
mediante consultas de existencia por lotes. Se cancelan trades y encuentros de
botín abiertos antes de readiness, pero los derechos ya generados sobreviven al
reinicio. Esos agregados temporales no quedan en el mundo activo.

El runtime admite hasta 65.536 instancias. Los snapshots raw heredados v1-v5
conservan el límite físico de 1 MiB; el snapshot v6 compatible y el v7 actual,
así como
cada nueva admisión/confirmación, admiten hasta 2 MiB. La suma de fichas de
propietarios
inspeccionadas sigue limitada a 1 MiB y cada ficha a 64 KiB. SQLite y
PostgreSQL esquema v12 guardan el snapshot como `TEXT` opaco dentro de la
transacción existente. La migración v10→v11 aumenta de forma transaccional la
restricción física del singleton runtime a 2 MiB y conserva versión, JSON,
secuencia cubierta y fecha; no convierte el JSON ni crea otra autoridad. No hay
tabla espejo/escrow/loot, doble escritura ni `jsonb`.

Tras una futura release estable, renombrar un ID requerirá alias, tombstone o
migración explícita. Nunca recicles un ID publicado para otro significado.
