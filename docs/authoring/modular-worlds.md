# Proyectos, áreas y composición

La fuente modular vigente es un manifiesto **WorldProject v2** con ficheros
**AreaPack v8** estrictos. `tiamud-world compose` valida el grafo completo y
produce un documento **WorldPack v9** canónico.

Usa [`worlds/lantern-square.project.json`](../../worlds/lantern-square.project.json)
y [`worlds/areas/square.area.json`](../../worlds/areas/square.area.json) como
ejemplos revisados.

## Ownership y dependencias

Un área posee cada sala, prototipo e instancia inicial de objeto, NPC, tabla y
fuente de botín, afijo/pool, habilidad, emote, entrada de progresión, medio,
script y prueba que declara. Los IDs estables son únicos globalmente en el
proyecto compuesto. Un par prototipo/instancia normalizado desde legacy puede
conservar la misma cadena porque ocupa espacios tipados distintos.

Un `entityMedia` de objeto depende del área propietaria del prototipo.
`roomPerception.hiddenItems` depende del área propietaria de la instancia
inicial concreta. Así todas las unidades visibles reutilizan una presentación
sin confundirla con visibilidad por instancia.

`dependsOn` debe nombrar cada área referenciada por datos estructurados. Se
permiten ciclos, por ejemplo dos salidas recíprocas. Una arista no declarada,
un destino ausente, doble ownership o colisión entre clases de ID impide
componer. El código Lua es opaco para el analizador y no crea una dependencia
verificable.

En botín, las dependencias cruzadas incluyen el prototipo de cada entrada, el
NPC de una fuente y su tabla. Tabla y fuente tienen ownership separado. Cada
spawn admite una sola fuente, toda tabla debe usarse y las instancias fijas
`npc_spawn` más el peor caso repetible no superan 64 raíces por derrota. Un
sello añade arista a su tabla; `affixPoolId`, al pool; y cada fila de pool, al
afijo. Una tabla sellada no puede ser fuente NPC directa. Las salas bloqueadas
por una habilidad de comunicación también crean aristas auditadas.

```json
{
  "schemaVersion": 8,
  "id": "north-gate",
  "dependsOn": ["square"],
  "itemPrototypes": [],
  "initialItemInstances": [],
  "rooms": [],
  "npcPrototypes": [],
  "npcSpawns": [],
  "lootTables": [],
  "lootSources": [],
  "affixes": [],
  "affixPools": [],
  "abilities": [],
  "socialEmotes": [],
  "entityMedia": []
}
```

Los campos desconocidos fallan cerrados. Mantén un AreaPack como frontera
coherente de ownership, no como servicio o shard.

AreaPack v1–v7 normaliza a v8 sin inventar emotes; v1–v6 tampoco inventa
comunicación, v1–v5 sellos/afijos y v1–v4 botín. Declarar campos posteriores en
un esquema anterior se rechaza. Títulos nativos y variantes admiten hasta 128
bytes UTF-8 seguros; descripciones, hasta 8.192.

## Composición

```sh
cargo run --locked -p world-pack --bin tiamud-world -- compose \
  worlds/lantern-square.project.json \
  --output build/worlds/lantern-square.candidate.world.json
```

La composición comprueba tamaños, rutas regulares seguras, assets locales,
digests de fuente, dependencias declaradas, referencias globales y WorldPack
final. Escribe atómicamente y no reemplaza una salida existente.

`CompositionReport` registra digests, ownership y referencias cruzadas. El
artefacto plano no conserva identidad de área. El orden sin semántica se
canoniza para que diff y reproducibilidad sean útiles.

Esto es autoría multipack, no sharding. La ruta monolítica compone todo sin
conexión y carga un WorldPack. La ruta local 6A publica bundles; 6B carga áreas
bajo demanda dentro de una generación; 6C puede sustituir solo la generación
completa mediante staging privado, cero sesiones/leases y CAS durable. No
existen despliegue por área, descarga, LRU ni servicios independientes.

Lantern Square compone tres áreas en tres salas, diez prototipos, nueve
instancias iniciales, dos tablas, una fuente, un afijo, un pool, una telepatía
directa y un emote. El artefacto canónico de 53.567 bytes tiene SHA-256
`9a75894a9e2e0c48d20257633f323d52bbe78b9db106de8c497f4784392b5e75`.

## Autoría WorldPack directa

Un WorldPack v9 monolítico sigue siendo válido, pero AreaPack es preferible con
varios builders o regiones. `tiamud-world build` valida y canoniza un documento
directo:

```sh
cargo run --locked -p world-pack --bin tiamud-world -- build \
  draft.world.json --output build/worlds/draft.canonical.world.json
```

No mezcles a mano artefactos generados. Cambia sus fuentes propietarias,
compón una candidata y revisa el diff semántico.
