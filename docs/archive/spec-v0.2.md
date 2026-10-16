<a id="tiamud-motor-mud-multimedia-browser-first"></a>

# Tiamud: motor MUD multimedia orientado al navegador

## Especificación técnica propuesta v0.2

- Estado: especificación histórica con estado implementado actualizado.
- Fecha original: 2026-07-15; nota de estado actualizada: 2026-07-17.
- Prioridad: ejecutable en un único equipo sin infraestructura externa.
- Alcance: Tiamud es un motor nuevo inspirado por EnvyMUD, Merc y otros MUD de su época, no una traducción de su código.

> Nota histórica: este documento conserva la propuesta previa a la entrega alfa
> y no es el contrato vigente. Tiamud es el nombre definitivo. La
> [especificación v0.3](../spec-v0.3.md), la
> matriz de completitud, la memoria y los ADR
> describen el baseline actual, incluido snapshot v8/esquema SQL v15 y el estado
> parcial de la retirada reversible 6D.

## 1. Decisión principal

El proyecto se implementará como un motor nuevo con nombre propio: Tiamud. EnvyMUD, Merc y otros MUD de su época servirán como referencias de experiencia, mecánicas y compatibilidad, pero no como base técnica.

Se desarrollarán por separado:

1. El motor nuevo.
2. Un importador de áreas y datos históricos.
3. Pruebas de compatibilidad de comandos y comportamiento.
4. Adaptadores para navegador, Telnet y clientes MUD.

La experiencia oficial dará prioridad al navegador, pero el juego seguirá siendo completamente utilizable mediante texto.

> Prioridad al navegador, sin depender exclusivamente de él. Prioridad al texto, sin limitarse exclusivamente a él.

## 2. Objetivos

### 2.1 Objetivos de producto

- Entrar a jugar desde un enlace sin instalar un cliente.
- Mantener la inmediatez, profundidad textual y capacidad de automatización asociadas a los MUD.
- Ofrecer imágenes, mapas, audio y controles visuales como mejoras opcionales.
- Permitir jugar desde Telnet, TinTin++, Mudlet y otros clientes compatibles.
- Funcionar bien en escritorio, móvil, teclado y lector de pantalla.
- Facilitar la creación, prueba y publicación de mundos.
- Poder importar contenido histórico cuando el operador tenga derechos para utilizarlo.

### 2.2 Objetivos técnicos

- Arrancar localmente con un solo ejecutable y un directorio de datos.
- No requerir Docker, Kubernetes, Redis, S3 ni servicios de IA. PostgreSQL es
  el perfil recomendado para un servidor público nuevo en un solo equipo; el
  arranque local sin servicios conserva SQLite.
- Mantener un núcleo autoritativo independiente de cualquier cliente.
- Usar colas y límites acotados; ningún cliente lento debe agotar el servidor.
- Mantener imágenes y transformaciones fuera del bucle de simulación.
- Permitir evolución futura sin diseñar prematuramente una arquitectura distribuida.
- Producir compilaciones reproducibles y actualizaciones migrables.

## 3. No objetivos iniciales

Quedan fuera del producto mínimo viable (MVP):

- Kubernetes y orquestación de clústeres.
- Microservicios.
- Redis, Kafka, NATS o buses externos.
- Registro del estado como una secuencia completa de eventos.
- IA obligatoria o NPC controlados permanentemente por un modelo de lenguaje de gran tamaño.
- Composición de mapas de bits en tiempo real.
- Vídeo.
- Capas gráficas complejas con NPC posicionados.
- Complementos no confiables ejecutándose dentro del servidor.
- Un formato gráfico universal nuevo.
- Una bifurcación propia de TinTin++.
- Escalado horizontal de un mismo mundo.

## 4. Licencia y relación con EnvyMUD

Antes de reutilizar código o contenido debe revisarse la licencia exacta de la distribución correspondiente. La relicencia de DikuMUD no relicencia automáticamente Merc, EnvyMUD ni otras obras derivadas.

Política propuesta:

- No copiar código fuente de EnvyMUD al nuevo motor.
- No copiar textos, áreas, ayudas ni recursos sin verificar sus derechos.
- Publicar el importador sin incluir contenido histórico.
- Conservar los VNUM importados como alias, no como identidad canónica.
- Usar el nombre Tiamud y completar su comprobación de disponibilidad antes de una publicación.
- Licenciar por separado motor, herramientas, mundo de ejemplo y recursos multimedia.

Licencia recomendada para el motor: MPL-2.0 si se desea que las modificaciones al núcleo sigan siendo compartibles sin impedir mundos privados. Apache-2.0 es una alternativa si se prioriza la máxima adopción.

> Decisión final posterior: el [ADR 0004](../decisions/0004-open-source-licensing.md)
> adoptó `MIT OR Apache-2.0` para el software y `CC-BY-4.0` para el contenido
> original. Esta recomendación se conserva como contexto histórico de la
> propuesta.

## 5. Perfil de despliegue prioritario

### 5.1 Perfil A: desarrollo o partida privada

```text
tiamud-server
  ├── HTTP + WebSocket en 127.0.0.1:8080
  ├── Telnet plaintext expreso en 127.0.0.1:4000, o TLS
  ├── cliente web y WorldPack revisado embebidos en el ejecutable
  ├── SQLite en data/tiamud.db
  ├── WorldPack externo opcional mediante TIAMUD_WORLD_PACK
  └── registros estructurados en stdout/stderr
```

Requisitos externos: ninguno.

### 5.2 Perfil B: servidor público en un único equipo

```text
Internet
  │
  ├── HTTPS/WSS ──► Caddy opcional ──► tiamud-server
  └── Telnet TLS ─────────────────────► tiamud-server

data/
  ├── assets/
  └── worlds/

systemd-journald
  └── captura stdout/stderr y aplica retención

PostgreSQL en el mismo equipo
  ├── base autoritativa
  └── backup, monitorización y recuperación nativos
```

Caddy o un servidor intermediario inverso equivalente simplifica HTTPS y certificados, pero no forma parte del motor ni es necesario para jugar en local.

El modo Telnet sin cifrar no forma parte de la superficie pública: solo se
habilita expresamente en loopback o una LAN confiable. Toda credencial Telnet
expuesta a Internet requiere el listener TLS directo.

El proyecto proporcionará:

- Unidad de `systemd` de ejemplo.
- Configuración mínima de Caddy.
- Comando de copia de seguridad.
- Comando de comprobación de salud.
- Guía para actualizar el binario y revertirlo.

### 5.3 Perfil C: crecimiento futuro

> **Estado implementado posterior.** PostgreSQL ya no pertenece a este perfil
> futuro: el adaptador de esquema v9, la selección exclusiva del sistema de
> persistencia y el traslado sin conexión desde SQLite están implementados. Es el perfil recomendado
> para instalaciones públicas nuevas en un solo equipo. SQLite sigue siendo el
> fallback sin servicios y la ruta heredada; no existe doble escritura.

Solo se implementará si las métricas lo justifican:

- Almacenamiento de objetos o CDN en lugar del sistema de archivos.
- Pasarela de conexiones separada del proceso de mundo.
- Varias instancias o mundos en procesos distintos.

La posibilidad de este perfil influye en las interfaces internas, pero no añade dependencias al MVP.

## 6. Arquitectura de un solo proceso

```text
┌───────────────────────────────────────────────────────────────┐
│                       tiamud-server                           │
│                                                               │
│  ┌─────────────┐  ┌──────────────┐  ┌─────────────────────┐  │
│  │ HTTP + WSS  │  │ Telnet/TLS   │  │ Admin./salud local  │  │
│  └──────┬──────┘  └──────┬───────┘  └──────────┬──────────┘  │
│         └────────────────┬┴──────────────────────┘             │
│                          ▼                                    │
│                  Enrutador de acciones                        │
│                          │                                    │
│                          ▼                                    │
│             Actor de mundo / núcleo de simulación             │
│                          │ DomainEvents                        │
│                          ▼                                    │
│              Percepción + proyección de vista                 │
│                   ┌──────┴─────────┐                          │
│                   ▼                ▼                          │
│              Eventos web      Texto/GMCP                      │
│                                                               │
│  ┌────────────────┐  ┌─────────────────┐  ┌────────────────┐ │
│  │ PostgreSQL o   │  │ Registro mundo  │  │ Recursos estát.│ │
│  │ SQLite (uno)   │  │ guiones/paquetes│  │ y cliente web  │ │
│  └────────────────┘  └─────────────────┘  └────────────────┘ │
└───────────────────────────────────────────────────────────────┘
```

Un proceso no implica un único hilo. El entorno de ejecución puede usar varios hilos para red, base de datos y tareas auxiliares, mientras el estado autoritativo de cada mundo conserva un único escritor lógico.

## 7. Capas del sistema

### 7.1 Definición del mundo

Contiene datos versionados e inmutables durante una publicación:

- Habitaciones y topología.
- Prototipos de objetos y NPC.
- Reglas, habilidades y tablas.
- Guiones de automatización.
- Definiciones de escenas.
- Referencias lógicas a recursos multimedia.
- Ayudas y localización.

### 7.2 Estado de ejecución

Contiene:

- Personajes e inventarios.
- Instancias de objetos y NPC.
- Combate y efectos.
- Clima y tiempo.
- Estados de misiones.
- Apariciones y temporizadores.
- Canales, grupos y sesiones.

### 7.3 Percepción

Determina qué conoce un personaje:

- Luz, ceguera y sentidos.
- Invisibilidad y ocultación.
- Puertas y salidas secretas.
- Estado de misiones.
- Instancia del mundo.
- Clasificación de contenido y preferencias.
- Información administrativa.

Todo filtrado se realiza antes de serializar, registrar o renderizar una vista de jugador.

### 7.4 Presentación

Añade pistas no autoritativas:

- Fondo visual.
- Punto focal.
- Paleta o tema.
- Atmósfera.
- Transición.
- Audio ambiental.
- Arte de celdas opcional.

### 7.5 Transporte

Convierte una vista ya autorizada a:

- JSON sobre WebSocket.
- Texto plano.
- ANSI.
- GMCP.
- MXP opcional futuro.

## 8. Modelo de concurrencia

Cada mundo o instancia principal tendrá un actor autoritativo:

1. Las conexiones validan y convierten entrada en `PlayerAction`.
2. Una cola `mpsc` acotada entrega acciones al mundo.
3. El mundo procesa acciones de forma ordenada.
4. Las reglas producen `DomainEvent`.
5. La proyección genera `ViewEvent` por sesión.
6. Cada conexión mantiene su propia cola de salida acotada.

Prioridades de salida:

```text
0: control de conexión y seguridad
1: resultado de acciones y texto crítico
2: combate, mensajes y conversación
3: estado de la interfaz de usuario
4: visuales y audio
5: precarga
```

Al aplicar presión:

- Se coalescen actualizaciones de indicadores vitales y estado visual.
- Se descartan precargas.
- Se reemplazan visuales obsoletos por el último.
- Nunca se descarta silenciosamente el resultado de un comando aceptado.
- Si la cola crítica no puede avanzar, se desconecta al cliente lento de forma explícita.

<a id="9-stack-del-servidor"></a>

## 9. Tecnologías del servidor

### 9.1 Elección principal

- Rust estable.
- Tokio para red, temporizadores y tareas.
- Axum para HTTP y WebSocket.
- rustls para TLS directo cuando se configure.
- serde para serialización.
- rusqlite con una conexión poseída por una tarea dedicada.
- Adaptador PostgreSQL de esquema v9 con conexión TLS seleccionable.
- tracing para registros y trazas.
- clap para la interfaz de línea de comandos.

### 9.2 Motivos

- Un único binario transportable.
- Bajo consumo en reposo.
- Seguridad de memoria en analizadores de red.
- Buen control de límites y contrapresión.
- Sin recolector de basura compartiendo tiempo con el ciclo del mundo.
- Posibilidad de reutilizar tipos entre servidor, compiladores y herramientas.

### 9.3 Alternativas

Si el equipo no dispone de experiencia suficiente en Rust, Go es la alternativa preferida. Evennia/Python es una opción válida para validar rápidamente un único juego, pero no sería la base predeterminada de este motor nuevo.

## 10. Persistencia local

> **Estado implementado posterior.** El perfil recomendado para una instalación
> pública nueva en un solo equipo es PostgreSQL. El ejecutable sigue arrancando
> sin variables con SQLite en `data/tiamud.db`, que conserva el desarrollo sin
> servicios y la compatibilidad de instalaciones anteriores. Ambos adaptadores
> comparten el esquema v9; el proceso abre exactamente uno. Esta recomendación
> operativa no añade Kubernetes ni cambia el actor autoritativo.

SQLite fue la base de datos predeterminada de la propuesta original y sigue
siendo la única opción que no necesita un servicio de base de datos separado.

Configuración:

- Modo WAL.
- Claves foráneas activadas.
- `busy_timeout` acotado.
- Migraciones incluidas en el binario.
- Transacciones para cambios jugables importantes.
- Puntos de control gestionados.
- Integridad comprobable mediante la interfaz de línea de comandos.

Datos persistidos inmediatamente:

- Cuenta y credenciales.
- Propiedad o pérdida de objetos.
- Economía.
- Avance de personaje.
- Resultado de misiones.
- Acciones administrativas.

Datos que pueden agruparse o reconstruirse:

- Posición visual temporal.
- Último estado mostrado.
- Precargas.
- Métricas agregadas.

Comandos operativos del perfil SQLite local/heredado:

```text
tiamud-admin migrate --database <db> --backup <safety-backup>
tiamud-admin check
tiamud-admin backup create --database <db> --output <backup>
tiamud-admin backup verify --backup <backup>
tiamud-admin backup restore --backup <backup> --database <new-db>
```

Estos subcomandos de `tiamud-admin` no implementan respaldo PostgreSQL. El
perfil PostgreSQL usa `pg_dump`/restore o el procedimiento nativo de respaldo
físico y recuperación elegido por el operador, con ejercicios periódicos de
restauración.

No se hacen escrituras dobles ni replicación artesanal entre SQLite y
PostgreSQL. La interfaz estrecha de persistencia y el adaptador PostgreSQL ya
están implementados. Cada instalación tiene un único sistema autoritativo; el
comando `tiamud-migrate-postgres` copia offline una instantánea SQLite actual a
un destino PostgreSQL vacío. Los datos que el servidor consulta siguen siendo
relacionales. Las instantáneas de personaje y mundo, que se cargan como una
unidad y no se consultan ad hoc, son JSON acotado almacenado en columnas `TEXT`
en ambos adaptadores; PostgreSQL no usa `jsonb` para ellas.

## 11. Cliente web

<a id="111-stack"></a>

### 11.1 Tecnologías

- TypeScript.
- Svelte.
- Vite.
- CSS propio con variables de tema.
- Un trabajador de servicio pequeño mediante la API `Service Worker`.
- Playwright para pruebas.

El resultado compilado se incrustará en el ejecutable del servidor. No habrá un entorno de ejecución de Node en producción.

### 11.2 Interfaz MVP

- Inicio de sesión y selección de personaje.
- Flujo narrativo.
- Línea de comandos con historial y autocompletado.
- Acciones contextuales accesibles.
- Título, descripción e ilustración de escena.
- Salidas y navegación.
- Indicadores vitales.
- Inventario y equipo.
- Los comandos de conversación `chat`, `tell`, `who` e `ignore`.
- Mapa básico.
- Reconexión y reanudación.
- Preferencias de imágenes, audio, movimiento y consumo de datos.

### 11.3 Diseño adaptable

Escritorio:

```text
┌──────── mapa/estado ───────┬──── escena y narrativa ────┬── inventario ──┐
│                            │                             │                │
└────────────────────────────┴──────── comando ────────────┴────────────────┘
```

Móvil:

- Escena y narrativa como vista principal.
- Paneles secundarios en pestañas o paneles deslizantes.
- Comando fijo sin cubrir la salida.
- Acciones frecuentes utilizables con una mano.

### 11.4 Rendimiento del cliente

- DOM virtualizado o recortado para el historial largo.
- Historial persistido limitado y configurable.
- Recursos multimedia adaptables.
- No descargar audio ni imágenes grandes cuando el usuario los desactive.
- Respetar `Save-Data` y preferencias equivalentes cuando estén disponibles.
- Objetivo inicial de JavaScript menor de 250 KiB comprimidos, excluyendo los recursos multimedia del juego.

### 11.5 Accesibilidad

- Objetivo WCAG 2.2 AA.
- Navegación completa por teclado.
- Flujo narrativo con `role="log"` y anuncios selectivos.
- No anunciar automáticamente cada línea repetitiva de combate.
- Texto alternativo contextual.
- Alto contraste y `forced-colors`.
- Tamaño de fuente configurable.
- `prefers-reduced-motion` y control manual.
- Modo lector de pantalla que desactiva arte de celdas y efectos decorativos.

La imagen no contendrá información jugable exclusiva.

## 12. Protocolo web

El navegador utilizará un protocolo propio sobre WebSocket, no GMCP encapsulado.

Estructura envolvente:

```json
{
  "v": 1,
  "type": "scene.snapshot",
  "seq": 1842,
  "id": "evt_01...",
  "requestId": null,
  "data": {}
}
```

Mensajes iniciales:

```text
client.hello
server.welcome

command.submit
action.perform
action.result

scene.snapshot
scene.patch
entity.upsert
entity.remove
vitals.update
inventory.snapshot
inventory.patch
map.snapshot
message.append

sync.request
sync.snapshot
server.notice
```

Reglas:

- Subprotocolo WebSocket `tiamud.v1`.
- JSON y UTF-8 en v1.
- Máximo inicial de 64 KiB por mensaje.
- Secuencia monotónica por sesión.
- `requestId` generado por el cliente para acciones.
- Caché breve de resultados para evitar duplicados tras reconectar.
- Retención breve de eventos para reanudación.
- Instantánea completa si falta parte de la secuencia.
- Latido de conexión y tiempo de espera.
- Validación mediante esquema y tipos del servidor.
- Autorización de cada acción.
- Los rechazos previos a `server.welcome` llevan un código `access.*`; solo
  `access.session_expired` y `access.session_invalid` invalidan la credencial.

Los botones envían acciones estructuradas:

```json
{
  "v": 1,
  "type": "action.perform",
  "requestId": "cmd_27",
  "data": {
    "action": "item.take",
    "target": "entity_51"
  }
}
```

La entrada libre sigue disponible:

```json
{
  "v": 1,
  "type": "command.submit",
  "requestId": "cmd_28",
  "data": {"text": "get sword"}
}
```

Ambas rutas terminan en la misma acción de dominio.

## 13. Telnet y clientes MUD

### 13.1 Compatibilidad mínima

Orden propuesto:

1. ECHO, SGA y EOR.
2. TTYPE y MTTS.
3. NAWS.
4. CHARSET/UTF-8.
5. GMCP.
6. MSSP.
7. Telnet TLS.
8. MCCP2.
9. MXP o MSLP opcional.

La negociación implementará el método Q de RFC 1143 y límites estrictos para subnegociaciones.

### 13.2 GMCP

Se reutilizarán paquetes conocidos cuando encajen:

- `Core`.
- `Room.Info`.
- `Char.Vitals`.
- `Char.Items`.
- `Comm`.
- `Group`.
- `Client.Media`.

La implementación ya anuncia `Tiamud.Character 1` para proyectar de forma
aditiva la ficha autorizada. La ficha ampliada reutiliza ese paquete existente;
no crea un segundo paquete GMCP de personaje.

El espacio de nombres propio se limitará inicialmente a:

```text
Tiamud.Scene 1
Tiamud.Assets 1
```

La lista anterior conserva el alcance multimedia de la propuesta original; no
pretende excluir el paquete `Tiamud.Character 1` ya implementado para estado
jugable.

MTTS será la fuente para ANSI, UTF-8, 256 colores, color verdadero y lector de pantalla. Solo capacidades no cubiertas, como un panel multimedia específico, necesitarán negociación propia.

### 13.3 TinTin++

No se hará una bifurcación en el MVP.

Se proporcionará:

- Guion de automatización oficial para GMCP.
- Indicadores vitales y estado.
- Automapa.
- Alias y activadores de ejemplo.
- Manejo de URLs de escena sin descarga automática obligatoria.

### 13.4 Mudlet

Se priorizará un paquete oficial capaz de:

- Mostrar la imagen de escena.
- Mantener mapa e indicadores vitales.
- Almacenar recursos multimedia en caché.
- Respetar preferencias multimedia.
- Recurrir a GMCP estándar si el paquete no está instalado.

### 13.5 Arte y gráficos de terminal

- Texto y ANSI son la base.
- El arte de celdas requerirá activación expresa.
- Kitty, SIXEL e iTerm2 serán experimentales y requerirán activación expresa.
- Nunca se activarán solo por el valor de `TTYPE`.

## 14. Modelo semántico de escena

```text
SceneView
  viewId
  revision
  title
  prose[]
  notices[]
  exits[]
  entities[]
  affordances[]
  mood[]
  environment
  map
  presentation
    backdrop
    focalPoint
    palette
    transition
```

Reglas:

- `viewId` es opaco y específico de la vista autorizada.
- No se envían VNUM ni IDs internos salvo necesidad documentada.
- Entidades, salidas y acciones ya están filtradas.
- La revisión pertenece a la vista, no necesariamente a la habitación global.
- Los visuales estáticos representan ambiente, no el estado autoritativo completo.
- NPC, puertas y objetos dinámicos permanecen en texto y datos estructurados durante el MVP.

Las actualizaciones parciales serán tipadas. No se utilizará una lista `changed` que duplique los datos ni el formato JSON Patch genérico en v1.

## 15. Variantes y condiciones

Las condiciones de contenido usan un árbol de sintaxis abstracta (AST) declarativo y validado:

```json
{
  "all": [
    {"eq": ["world.time.period", "night"]},
    {"in": ["world.weather", ["rain", "storm"]]}
  ]
}
```

No se ejecutan expresiones arbitrarias para seleccionar una variante.

El compilador detectará:

- Campos y operadores desconocidos.
- Tipos incompatibles.
- Prioridades empatadas.
- Ausencia de variante por defecto.
- Condiciones imposibles conocidas.
- Referencias rotas.
- Recursos multimedia ausentes.
- Posibles fugas de contenido secreto.

<a id="16-world-packs-e-importación"></a>

## 16. Paquetes WorldPack e importación

> **Estado implementado posterior.** El servidor no carga esta estructura de
> directorios: consume un único WorldPack v3 plano y validado, sin conservar
> una entidad `area` en el entorno de ejecución ni componer varios paquetes durante
> el arranque. La herramienta implementada `tiamud-world compose` fusiona sin
> conexión un manifiesto de proyecto estricto y entre 1 y 256 módulos
> AreaPack v2 en ese artefacto final; valida dependencias declaradas,
> referencias y alcanzabilidad globales, colisiones de ID, recursos y límites.
> La estructura y los comandos que siguen se conservan como propuesta
> histórica. La [guía actual de autoría](../world-authoring.md) documenta el
> contrato real, el ejemplo modular, las herramientas, los límites y la
> publicación inmutable.

Estructura propuesta:

```text
world/
  schema/
  core/
  areas/
    midgaard/
      area.json
      rooms.json
      mobs.json
      objects.json
      quests.json
      scenes.json
      scripts/
```

IDs canónicos:

```text
midgaard:room/plaza
midgaard:mob/cityguard
midgaard:item/fountain-key
```

Alias histórico:

```json
{"legacyAliases": ["envy:vnum:3001"]}
```

Herramientas integradas en el mismo binario o distribuidas junto a él:

```text
tiamud-admin world validate
tiamud-admin world build
tiamud-admin world diff
tiamud-admin world preview
envy-import convert
tiamud-admin world test
```

El importador produce un informe de:

- Entradas convertidas.
- Referencias rotas.
- Indicadores desconocidos.
- Colisiones.
- Codificaciones dudosas.
- MOBPROG o guiones no compatibles.
- Referencias a contenido cuya licencia no está declarada.

### 16.1 Entrega v2 histórica de ficha, arquetipos y habilidades

> Este apartado conserva la terminología de la entrega v2 que siguió a la
> propuesta original. El contrato vigente está en WorldPack v3: linaje,
> trasfondo opcional, clase base/avanzada, XP global, reservas de puntos,
> talentos con rango y promoción atómica. `archetype`/arquetipo solo se acepta
> en límites explícitos de normalización. Véanse la
> [especificación v0.3](../spec-v0.3.md) y el
> [diseño de progresión v3](../design/character-progression-v3.md).

El `gameplay` del WorldPack v2 admite de forma opcional un catálogo acotado de
arquetipos, habilidades y repertorios de NPC. Que sean opcionales conserva los
WorldPacks y personajes anteriores: cuando no existen, el runtime mantiene la
ficha y la fórmula de nivel heredadas en vez de rechazar el mundo.

Un arquetipo declara en datos:

- atributos iniciales —con `max_health`, `strength` y `defence` como mínimo
  interoperable— y estadísticas adicionales extensibles;
- ranks iniciales de skills, pools de recursos, regeneración por tick y
  resistencias;
- habilidades iniciales; y
- hitos contiguos de nivel 2 a 100 con umbral acumulado de XP, aumentos de
  atributo, skill y capacidad de recurso, y desbloqueos.

Una única definición de habilidad representa ataques, técnicas marciales,
magia y acciones innatas de NPC. Declara costes, objetivo, cooldown, daño o
curación escalable y estados temporales acotados. El dominio implementa daño
`physical`, `arcane`, `fire`, `frost` y `lightning`, resistencia de 0 a 90 %,
modificadores de atributo, regeneración, daño periódico y stun. La habilidad
del jugador aplica además un cooldown global de un tick lógico.

Los NPC reciben pools, regeneración, resistencias y una lista de habilidades
desde WorldPack. En cada turno de combate eligen de forma determinista una
opción utilizable mediante reglas acotadas y recurren al ataque básico cuando
ninguna sirve. Esta “IA” de combate no usa un modelo generativo ni un servicio
externo.

El actor valida aprendizaje, objetivo, recursos, cooldowns, recuperación y
capacidad de estados antes de confirmar. XP, nivel, ficha, recursos,
habilidades, cooldowns, resistencias y estados activos forman parte del estado
durable. Navegador y Telnet comparten esta ruta de dominio; el navegador ofrece
ficha y controles accesibles, y el texto acepta `ability`, `skill` y `cast`.
La [guía de autoría](../world-authoring.md) contiene el esquema y los límites
exactos; el contrato de navegador se conserva en
[`tiamud.v1`](../protocol/tiamud-v1.md).

No existe todavía un sistema general de entrenamiento de skills. Su progresión
configurable procede de los hitos authored; además, la victoria en combate
conserva el incremento heredado actual del rank `combat`.

Durante la creación web, el catálogo público muestra solo label y descripción
del arquetipo. La ficha inicial completa se proyecta después de entrar. Una
previsualización de atributos, recursos y habilidades antes de crear requiere
ampliar el contrato y queda pendiente.

Los IDs publicados de arquetipo, habilidad, recurso y estadística son identidad
durable de contenido. Al restaurar, el runtime filtra estado efímero desconocido
y elimina o ignora con seguridad habilidades aprendidas que ya no existen en el
pack activo. Eliminar contenido falla así de forma segura, pero renombrarlo no
es transparente: aliases y migraciones explícitas de esos IDs siguen siendo
trabajo futuro.

<a id="17-scripting"></a>

## 17. Automatización mediante guiones

La mayor parte del mundo debe poder expresarse mediante datos, condiciones y acciones predefinidas.

Para comportamiento especial se propone Lua embebido y versionado, con estas restricciones:

- Sin acceso al sistema de archivos.
- Sin red.
- Sin ejecución de procesos.
- Sin carga de bibliotecas nativas.
- API basada en capacidades explícitas.
- Límite de memoria por ejecución o contexto.
- Límite de instrucciones/tiempo.
- Número máximo de eventos encadenados.
- Semillas controlables para pruebas.

Los guiones creados por los autores del mundo se consideran contenido confiable del operador, pero aun así se limitan para evitar errores y saturación accidental.

La API incluirá inicialmente puntos de extensión como:

```text
on_enter
on_leave
on_look
on_use
on_talk
on_combat_start
on_defeat
on_timer
on_quest_event
```

<a id="18-assets-y-publicación-multimedia"></a>

## 18. Recursos y publicación multimedia

> **Estado implementado posterior.** La herramienta real `tiamud-assets`
> procesa las imágenes sin conexión con bibliotecas Rust, publica bundles
> `assets/<sha256>/manifest.json` con variantes de 640 y 1280 píxeles, y exige
> procedencia y revisión. El servidor de referencia sirve la distribución web
> incrustada en el ejecutable; el registro de WorldPacks no copia multimedia.
> Las rutas fragmentadas y libvips descritos a continuación pertenecen a la
> propuesta histórica. Consulte la [guía actual de autoría y
> multimedia](../world-authoring.md).

### 18.1 Almacenamiento v1

Los recursos publicados residen en el sistema de archivos local:

```text
data/assets/sha256/ab/cd/<digest>.avif
data/assets/sha256/ab/cd/<digest>.webp
data/assets/sha256/ab/cd/<digest>.jpg
```

El servidor los sirve directamente con caché inmutable. S3 o CDN serán adaptadores futuros.

<a id="182-pipeline"></a>

### 18.2 Proceso de publicación

```text
fuente
  ↓
cuarentena y límites
  ↓
decodificación y recodificación aislada
  ↓
eliminación de metadatos
  ↓
variantes responsivas
  ↓
hash de cada salida
  ↓
manifiesto publicado
```

Se propone libvips para el procesamiento sin conexión.

Formatos:

- AVIF para navegador moderno.
- WebP como alternativa moderna.
- JPEG como compatibilidad.
- PNG únicamente cuando se requiera transparencia o compresión sin pérdida.

Anchuras iniciales:

```text
640, 1280
```

Se añadirán 320, 960 o 1920 únicamente cuando los perfiles de dispositivos y activos reales demuestren que compensan el coste de almacenamiento y compilación.

Presupuestos orientativos:

- Imagen habitual: 150–500 KiB.
- Imagen excepcional: hasta 2 MiB.
- Dimensión máxima publicada: 4096 × 4096.
- Carga de la fuente: configurable y procesada fuera del actor de mundo.

### 18.3 Manifiesto

Cada recurso lógico registrará:

- ID y revisión.
- Variantes y resúmenes criptográficos.
- Dimensiones.
- Punto focal.
- Color dominante o marcador de posición.
- Autor y licencia.
- Procedencia.
- Herramienta o modelo generativo si aplica.
- Clasificación de contenido.
- Fecha de revisión humana.

El texto alternativo contextual pertenece a la escena y localización, no necesariamente al archivo gráfico global.

### 18.4 Arte de celdas

Se definirá primero un modelo interno:

```text
CellFrame
  width
  height
  palette
  cells[glyph, foreground, background, attributes]
```

No se publicará EVC1 como estándar hasta disponer de al menos dos consumidores y casos reales que definan sus requisitos.

## 19. IA: opcional y fuera del camino crítico

El servidor funcionará completamente sin IA.

### 19.1 Usos recomendados

La IA encaja primero como herramienta editorial:

- Proponer borradores de descripciones.
- Sugerir texto alternativo.
- Revisar consistencia de un área.
- Detectar salidas o referencias sospechosas.
- Generar ideas para NPC o misiones.
- Crear imágenes durante la autoría.

Flujo obligatorio:

```text
sugerencia de IA
  ↓
borrador
  ↓
revisión humana
  ↓
world/assets build
  ↓
publicación
```

### 19.2 Ejecución local ligera

La integración se hará mediante un adaptador opcional, no enlazando un modelo dentro del servidor:

```text
ai.provider = "none"       # valor por defecto
ai.provider = "ollama"     # opcional
ai.provider = "llama.cpp"  # opcional
ai.provider = "compatible" # punto de acceso configurado por el operador
```

Para ejecución local se favorecerán modelos cuantizados pequeños, aproximadamente de 1B–4B parámetros, usados solo bajo demanda editorial.

El motor no descargará modelos automáticamente.

### 19.3 IA durante el juego

No forma parte del MVP. Si se experimenta posteriormente:

- Se ejecuta en una tarea auxiliar o proceso opcional.
- Nunca decide reglas, recompensas, daño, inventario ni permisos.
- Cola global y por jugador estrictamente acotada.
- Límite configurable de CPU, RAM, tokens y tiempo.
- Respuesta almacenable en caché cuando sea apropiado.
- Alternativa determinista si falla o está desactivada.
- Posibilidad de desactivar por mundo y por jugador.
- Etiquetado claro de contenido generado.

Un fallo del proveedor de IA no puede ralentizar movimiento, combate, conversación ni guardado.

### 19.4 Desarrollo asistido de forma transparente

El proyecto utiliza asistencia de IA en algunas tareas de desarrollo,
documentación, revisión editorial y creación visual. Esa asistencia se declara
en los registros de entrega o procedencia cuando es material. Una propuesta de
un modelo no constituye evidencia ni autoridad: una persona revisa el cambio y
se aplican los mismos validadores, pruebas y criterios de aceptación. Ningún
asistente de desarrollo es necesario para compilar, ejecutar o jugar Tiamud.

## 20. Audio y animación

- Audio desactivado por defecto hasta que el usuario lo habilite.
- El navegador debe recibir una interacción antes de intentar reproducir audio.
- En clientes MUD se reutilizará `Client.Media` cuando sea posible.
- El volumen de música, ambiente y efectos se controla por separado.
- `prefers-reduced-motion` y la preferencia manual pueden desactivar transiciones.
- Vídeo y escenas por capas quedan fuera del MVP.

## 21. Seguridad

### 21.1 Red y autenticación

- HTTPS/WSS en producción pública.
- Validación exacta de `Origin` para WebSocket.
- Cookies `HttpOnly`, `Secure` y `SameSite` cuando se use sesión web.
- Autorización en cada acción.
- Argon2id para contraseñas.
- Claves de acceso como mejora futura.
- Telnet TLS directo para cualquier listener público con credenciales.
- Telnet sin cifrar sólo mediante activación expresa, con advertencia y
  enlazado a loopback o una LAN confiable; nunca como endpoint público.

### 21.2 Límites

- Tamaño de mensaje WebSocket: 64 KiB inicialmente.
- Subnegociación Telnet acotada.
- Profundidad y tamaño JSON acotados.
- Frecuencia limitada de comandos y de intentos de inicio de sesión.
- Máximo de sesiones por cuenta e IP configurable.
- Colas acotadas en todas las fronteras.
- Tiempos de espera para la base de datos, los guiones, los recursos y el apagado.

<a id="213-contenido-y-assets"></a>

### 21.3 Contenido y recursos

- Sin SVG remoto en v1.
- Sin HTML o JavaScript arbitrario.
- Sin `data:` o `file:` URLs.
- Sin servidores externos no permitidos.
- MIME detectado por contenido y recodificación de imágenes.
- Metadatos eliminados.
- CSP y `X-Content-Type-Options: nosniff`.
- Procesamiento de cargas fuera del actor de mundo y con recursos limitados.

### 21.4 Moderación

Antes de una alfa pública:

- Funciones para ignorar, silenciar y bloquear.
- Reporte de usuarios y mensajes.
- Limitación de frecuencia de la conversación.
- Roles administrativos.
- Registro de auditoría.
- Bloqueos temporales y permanentes.
- Protección de datos sensibles en los registros.

## 22. Operación y observabilidad

Registros estructurados con rotación y salida legible para una instalación simple.

Métricas mínimas:

- Conexiones por transporte.
- Sesiones autenticadas.
- Latencia de acciones.
- Longitud de colas.
- Visuales coalescidos o descartados.
- Tiempo de ejecución de guiones y esperas agotadas.
- Transacciones y errores SQLite.
- Uso de memoria.
- Tiempo de ciclo o de procesamiento de acciones.

El punto de acceso de salud distinguirá:

```text
/health/live
/health/ready
```

Apagado ordenado:

1. Dejar de aceptar conexiones.
2. Informar a los clientes.
3. Detener nuevas acciones.
4. Drenar acciones aceptadas dentro de un tiempo de espera.
5. Persistir estado necesario.
6. Cerrar SQLite.
7. Permitir que el cliente reconecte al reiniciar.

No se implementará `copyover` tradicional.

## 23. Repositorio: propuesta histórica y realidad implementada

El borrador inicial proponía esta separación conceptual:

```text
apps/
  server/
  web/

crates/
  domain/
  world/
  perception/
  protocol/
  telnet/
  persistence-sqlite/
  scripting/
  assets/

tools/
  worldc/
  assetc/
  envy-import/
  bot-client/

world/
  schema/
  example/

docs/
  spec-v0.2.md
  protocol/
  operations/
```

Podrá entregarse como un único comando `tiamud-server`; la separación del repositorio es modularidad de desarrollo, no microservicios.

La disposición real consolidó esas fronteras en menos paquetes:

```text
apps/web/                    cliente Svelte
crates/mud-core/             dominio y percepción
crates/mud-server/           actor, adaptadores y servidor embebido
crates/telnet-codec/         parser Telnet sans-I/O
crates/persistence-*/        adaptadores SQLite y PostgreSQL
crates/world-pack/           modelo y herramientas de mundo
crates/{scripting,asset-pipeline,cell-frame,editorial-ai}/
crates/tiamud-admin/         operación SQLite
tools/envy-import/           importador histórico offline
worlds/                      fuente y pack revisado de ejemplo
```

El binario de producción incluye el build web y el WorldPack revisado; un pack
externo es opcional. Los registros estructurados salen por `stdout`/`stderr`
para que el supervisor aplique rotación y retención, no a `data/logs/`.

## 24. Pruebas

### 24.1 Dominio

- Pruebas deterministas de combate, movimiento y percepción.
- Pruebas basadas en propiedades de inventario, economía y efectos.
- Semillas reproducibles.
- Pruebas de invariantes después de cada acción.

### 24.2 Protocolos

- Pruebas mediante generación automatizada de entradas inválidas y limítrofes para Telnet e IAC.
- Negociaciones fragmentadas y concatenadas.
- JSON truncado, profundo y sobredimensionado.
- Reconexión y reanudación.
- Acciones duplicadas.
- Cliente lento y presión de salida.

### 24.3 Contenido

- Esquemas.
- Referencias.
- Habitaciones inalcanzables.
- Salidas no recíprocas cuando deban serlo.
- Variantes sin valor predeterminado.
- Texto alternativo ausente.
- Fugas de contenido oculto.
- Pruebas de referencia del renderizado de texto.

### 24.4 Web y accesibilidad

- Chromium, Firefox y WebKit mediante Playwright.
- Tamaños de ventana gráfica para móvil y escritorio.
- Navegación por teclado.
- Contraste y movimiento reducido.
- Lectores de pantalla mediante pruebas manuales periódicas.

### 24.5 Carga

- Cliente automatizado sin interfaz gráfica que conecta, autentica, mueve, habla y combate.
- Prueba prolongada de 24 horas.
- Clientes que reconectan repetidamente.
- Recursos ausentes o lentos.
- Guiones que consumen su cuota.

<a id="25-roadmap"></a>

## 25. Hoja de ruta

<a id="fase-0--decisiones-y-walking-skeleton-2-semanas"></a>

### Fase 0 — Decisiones y esqueleto funcional (2 semanas)

- Revisión legal y elección de licencia.
- Nombre definitivo elegido: Tiamud.
- Repositorio único para Rust y TypeScript.
- Un ejecutable con web embebida.
- SQLite y migración inicial.
- Una habitación compartida por navegador y Telnet.
- Cliente automatizado de prueba.

Criterio:

> `tiamud-server` permite entrar desde navegador y Telnet al mismo mundo sin servicios externos.

<a id="fase-1--vertical-browser-first-46-semanas"></a>

### Fase 1 — Vertical orientada al navegador (4–6 semanas)

> **Estado implementado posterior.** Además de este corte original, la primera
> ampliación social ya incorpora chat de grupo, revocación/expulsión/
> transferencia/disolución, `reply` en línea y suscripciones durables a
> `general`, `newbie` y `staff`. Navegador, texto Telnet y GMCP comparten la
> misma política; el canal `staff` exige rol Moderator/Admin. Correo sin
> conexión, telepatía definida por el mundo y capacidades avanzadas de staff
> siguen pendientes.

- Cuenta registrada y selección de personaje.
- Personaje.
- Movimiento y comandos `look`, `say`, `tell`, `who` y `help`.
- `scene.snapshot`.
- Imagen externa o local con alternativa textual.
- Reconexión e instantánea.
- Cliente adaptable.
- Texto Telnet limpio.

Criterio:

> Una pequeña aventura social se puede jugar desde navegador y terminal.

### Fase 2 — Bucle jugable (6–8 semanas)

> **Estado implementado posterior.** Este bucle incluye ahora arquetipos,
> ficha, XP e hitos de nivel, recursos, técnicas y magia unificadas, cooldowns,
> tipos de daño, resistencias, estados y selección determinista de habilidades
> de NPC, además del fallback para contenido heredado.

- Inventario y equipo.
- Combate.
- Habilidades y efectos.
- NPC y apariciones.
- Muerte y recuperación.
- Persistencia duradera.
- Primeros puntos de extensión para guiones.
- Indicadores vitales e inventario estructurados.

Criterio:

> El jugador puede explorar, combatir, saquear, progresar y volver tras reiniciar el servidor.

### Fase 3 — Contenido y multimedia (4–6 semanas)

- Paquetes WorldPack.
- `world validate/build/diff`.
- `asset build`.
- Variantes de imagen.
- Condiciones de escena.
- Importador histórico EnvyMUD/Merc parcial.
- Previsualización de texto, web y ANSI.

Criterio:

> Un área se valida, compila, publica y puede importarse parcialmente desde un formato histórico.

<a id="fase-4--alpha-pública-68-semanas"></a>

### Fase 4 — Alfa pública (6–8 semanas)

- Telnet TLS.
- GMCP estándar.
- Paquete Mudlet.
- Guion de automatización para TinTin++.
- Mapa y grupos.
- Moderación.
- Accesibilidad y móvil.
- Copia de seguridad y restauración probadas.
- Métricas y pruebas de carga.

Criterio:

> El servidor puede mantenerse públicamente en un único equipo con procedimientos documentados.

### Fase 5 — Beta (4–6 semanas)

- Publicación y reversión de mundos.
- Localización preparada.
- Mejoras de incorporación de nuevos jugadores.
- Refuerzo de seguridad.
- Documentación para creadores y operadores.
- PostgreSQL ya implementado y recomendado para servidores públicos nuevos en
  un solo equipo; almacenamiento de objetos o separación de procesos siguen
  sujetos a métricas.

## 26. Criterios globales de aceptación

1. El servidor local arranca con un único ejecutable.
2. No requiere conexión a Internet para iniciar y jugar en LAN/local.
3. Navegador y Telnet usan las mismas acciones de dominio.
4. El texto contiene toda la información jugable.
5. Una imagen ausente nunca bloquea una acción.
6. Ningún cliente recibe gráficos de terminal sin consentimiento.
7. No existen colas sin límite.
8. Una reconexión no duplica la última acción confirmada.
9. El cliente puede pedir una instantánea tras perder eventos.
10. Los IDs externos no revelan variantes ocultas.
11. SQLite puede respaldarse y restaurarse mediante la interfaz de línea de comandos.
12. El mundo puede actualizarse y revertirse de forma documentada.
13. Los guiones tienen cuotas de tiempo y recursos.
14. La IA puede estar completamente ausente.
15. Un fallo de IA no afecta al juego.
16. La instalación pública en un solo equipo está documentada.
17. El navegador funciona con teclado y lector de pantalla.
18. Telnet básico recibe texto limpio.
19. El proceso supera una prueba prolongada de 24 horas sin crecimiento continuo de memoria.
20. Cualquier paso hacia arquitectura distribuida exige métricas que lo justifiquen.

## 27. Decisiones abiertas en la propuesta original

- Lua frente a un lenguaje embebido puro Rust.
- ~~MPL-2.0 frente a Apache-2.0.~~ Resuelta por el ADR 0004 con la expresión
  dual `MIT OR Apache-2.0` para el software y `CC-BY-4.0` para contenido
  original.
- Método de emisión y renovación de certificados para Telnet TLS.
- ~~Primer mundo de ejemplo y licencia de su contenido.~~ Resuelta con Lantern
  Square y la licencia de contenido `CC-BY-4.0` documentada en
  `LICENSE-CONTENT.md` y el ADR 0004; el manifiesto de cada recurso conserva
  además su licencia y procedencia específicas.
- Objetivos de concurrencia realistas para el primer servidor público.
- Política de cuentas: contraseña, enlace de acceso y/o claves de acceso.
- Nivel de compatibilidad requerido con clientes distintos de Mudlet y TinTin++.

## 28. Regla de evolución

Se empezará con la solución local más sencilla que mantenga límites y fronteras correctos.

Una dependencia o servicio nuevo solo se añade cuando:

1. Resuelve un problema observado.
2. Existe una métrica o caso reproducible.
3. No puede resolverse razonablemente dentro del proceso actual.
4. Incluye migración, operación y reversión documentadas.

El objetivo no es construir infraestructura para un MMO hipotético, sino hacer un MUD moderno, rápido, mantenible y agradable de operar.

## 29. Decisiones cerradas el 15 de julio de 2026

Estas decisiones sustituyen cualquier alternativa contradictoria que permanezca en borradores anteriores:

> La decisión 5 conserva el punto de partida histórico. El
> [ADR 0006](../decisions/0006-character-progression-and-postgresql-first.md) la
> actualiza después de implementar y probar el adaptador PostgreSQL: PostgreSQL
> pasa a ser la recomendación para servidores públicos nuevos, aunque SQLite
> continúa como comportamiento sin configuración y fallback local/heredado.

1. Se construirá Tiamud como motor nuevo en Rust, inspirado por EnvyMUD, Merc y otros MUD de su época, pero sin copiar sus implementaciones históricas.
2. El producto dará prioridad al navegador y será completo mediante texto. WebSocket será el transporte principal del navegador; Telnet textual seguirá siendo jugable.
3. El despliegue de referencia será un único proceso en un equipo corriente. Docker será opcional y Kubernetes, Redis, S3 y microservicios no formarán parte de v1.
4. El núcleo será un actor propietario del mundo. Las acciones de dominio serán independientes de HTTP, WebSocket y Telnet.
5. SQLite en WAL será el sistema de persistencia predeterminado mediante rusqlite y una tarea dedicada. PostgreSQL podrá añadirse como sistema de persistencia alternativo, nunca como doble escritura obligatoria.
6. Las ilustraciones serán acompañamiento atmosférico. No contendrán información jugable exclusiva ni intentarán componer en tiempo real cada NPC u objeto.
7. El proceso sin conexión producirá AVIF como variante preferente para navegador, WebP como alternativa moderna y JPEG como compatibilidad. Las primeras anchuras serán 640 y 1280 píxeles.
8. Los fallos o ausencias de imágenes no bloquearán el juego. El procesamiento y la IA no se ejecutarán dentro del actor de mundo.
9. La IA será una ayuda declarada para desarrollo y, opcionalmente, para autoría ligera sin conexión. No será dependencia de ejecución y toda salida editorial requerirá revisión humana.
10. Las áreas históricas EnvyMUD/Merc legalmente utilizables servirán para ayudar a migrar contenido mediante el importador independiente sin conexión `envy-import`. WorldPack seguirá siendo el formato canónico de Tiamud.
11. La reconexión pedirá una instantánea autoritativa; v1 no implementará reproducción general de comandos ni eventos.
12. Las decisiones permanentes y el estado de implementación se conservarán dentro del repositorio, junto con esta especificación histórica versionada.

## 30. Decisiones cerradas el 16 de julio de 2026

1. La ficha de personaje, los arquetipos, la XP, los hitos de nivel, los pools
   de recursos y los desbloqueos se expresan como WorldPack opcional y acotado;
   los mundos antiguos conservan el comportamiento heredado.
2. Técnicas marciales, magia y acciones innatas de NPC comparten una definición
   de habilidad con costes, cooldown, objetivo, daño o curación y estados. Los
   jugadores tienen un cooldown global de un tick lógico.
3. Los tipos de daño y resistencias son reglas de dominio; los NPC eligen de
   forma determinista entre habilidades utilizables y recurren al ataque básico.
4. PostgreSQL es el perfil recomendado para una instalación pública nueva en un
   solo equipo. SQLite sigue siendo el arranque sin servicios y la ruta
   heredada. Solo se abre un backend y el cambio es offline, sin doble escritura.
5. El despliegue continúa siendo un monolito modular Rust en un host corriente;
   Kubernetes no es un requisito ni una capacidad que se dé por probada.
6. AVIF continúa como formato preferente, con WebP y JPEG de respaldo. El texto
   conserva toda información jugable cuando falla la multimedia.
7. La IA editorial es ligera, opcional y fuera del juego. La asistencia de IA
   durante el desarrollo se declara cuando es material y no sustituye revisión
   humana, validación ni pruebas.
8. PostgreSQL y SQLite son backends mutuamente exclusivos, sin doble escritura.
   Los datos consultables son relacionales; las instantáneas JSON acotadas se
   almacenan como `TEXT`, no `jsonb`. `tiamud-admin` respalda SQLite y PostgreSQL
   usa sus herramientas nativas.
9. La superficie Telnet pública requiere TLS directo. Plaintext solo se admite
   mediante activación expresa en loopback o una LAN confiable.
10. El catálogo web de creación muestra label y descripción; la ficha completa
    llega tras entrar y la previsualización de stats iniciales queda pendiente.
    `Tiamud.Character 1` es el paquete GMCP existente que proyecta la ficha
    ampliada.
11. Los IDs de contenido publicados son estables. La restauración filtra estado
    efímero desconocido y habilidades retiradas, pero los renombres funcionales
    aún necesitan aliases o una migración futura explícita.
12. El actor no publica un handle listo hasta restaurar y reproducir su estado;
    un snapshot inválido produce log estructurado y aborta el arranque antes de
    abrir HTTP o Telnet.
13. El checkpoint/rollback y la instantánea autoritativa de reconexión se
    conservan. El dirty tracking de personajes y los deltas de tick quedan como
    optimización single-host posterior, sin añadir otra autoridad ni servicios.
