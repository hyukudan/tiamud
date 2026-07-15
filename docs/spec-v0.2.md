# Tiamud: motor MUD multimedia browser-first

## Especificación técnica propuesta v0.2

- Estado: borrador para revisión.
- Fecha: 2026-07-15.
- Prioridad: ejecutable en un único equipo sin infraestructura externa.
- Alcance: Tiamud es un motor nuevo inspirado por EnvyMUD, Merc y otros MUD de su época, no una traducción de su código.

> Nota histórica: este documento conserva la propuesta previa a la entrega alpha. Tiamud es el nombre definitivo. La [matriz de completitud](project/COMPLETION_MATRIX.md) contrasta cada requisito con evidencia actual; la memoria, los ADR y los logs versionados conservan las decisiones y entregas.

## 1. Decisión principal

El proyecto se implementará como un motor nuevo con nombre propio: Tiamud. EnvyMUD, Merc y otros MUD de su época servirán como referencias de experiencia, mecánicas y compatibilidad, pero no como base técnica.

Se desarrollarán por separado:

1. El motor nuevo.
2. Un importador de áreas y datos históricos.
3. Pruebas de compatibilidad de comandos y comportamiento.
4. Adaptadores para navegador, Telnet y clientes MUD.

La experiencia oficial será browser-first, pero el juego seguirá siendo completamente utilizable mediante texto.

> Browser-first, no browser-only. Text-first, no text-only.

## 2. Objetivos

### 2.1 Objetivos de producto

- Entrar a jugar desde un enlace sin instalar un cliente.
- Mantener la inmediatez, profundidad textual y scripting asociados a los MUD.
- Ofrecer imágenes, mapas, audio y controles visuales como mejoras opcionales.
- Permitir jugar desde Telnet, TinTin++, Mudlet y otros clientes compatibles.
- Funcionar bien en escritorio, móvil, teclado y lector de pantalla.
- Facilitar la creación, prueba y publicación de mundos.
- Poder importar contenido histórico cuando el operador tenga derechos para utilizarlo.

### 2.2 Objetivos técnicos

- Arrancar localmente con un solo ejecutable y un directorio de datos.
- No requerir Docker, Kubernetes, Redis, PostgreSQL, S3 ni servicios de IA.
- Mantener un núcleo autoritativo independiente de cualquier cliente.
- Usar colas y límites acotados; ningún cliente lento debe agotar el servidor.
- Mantener imágenes y transformaciones fuera del bucle de simulación.
- Permitir evolución futura sin diseñar prematuramente una arquitectura distribuida.
- Producir compilaciones reproducibles y actualizaciones migrables.

## 3. No objetivos iniciales

Quedan fuera del MVP:

- Kubernetes y orquestación de clústeres.
- Microservicios.
- Redis, Kafka, NATS o buses externos.
- Event sourcing completo.
- IA obligatoria o NPC controlados permanentemente por un LLM.
- Composición raster en tiempo real.
- Vídeo.
- Capas gráficas complejas con NPC posicionados.
- Plugins no confiables ejecutándose dentro del servidor.
- Un formato gráfico universal nuevo.
- Un fork propio de TinTin++.
- Escalado horizontal de un mismo mundo.

## 4. Licencia y relación con EnvyMUD

Antes de reutilizar código o contenido debe revisarse la licencia exacta de la distribución correspondiente. La relicencia de DikuMUD no relicencia automáticamente Merc, EnvyMUD ni otras obras derivadas.

Política propuesta:

- No copiar código fuente de EnvyMUD al nuevo motor.
- No copiar textos, áreas, ayudas ni recursos sin verificar sus derechos.
- Publicar el importador sin incluir contenido histórico.
- Conservar los VNUM importados como alias, no como identidad canónica.
- Usar el nombre Tiamud y completar su comprobación de disponibilidad antes de una publicación.
- Licenciar por separado motor, herramientas, mundo de ejemplo y assets.

Licencia recomendada para el motor: MPL-2.0 si se desea que las modificaciones al núcleo sigan siendo compartibles sin impedir mundos privados. Apache-2.0 es una alternativa si se prioriza la máxima adopción.

> Decisión final posterior: el [ADR 0004](decisions/0004-open-source-licensing.md)
> adoptó `MIT OR Apache-2.0` para el software y `CC-BY-4.0` para el contenido
> original. Esta recomendación se conserva como contexto histórico de la
> propuesta.

## 5. Perfil de despliegue prioritario

### 5.1 Perfil A: desarrollo o partida privada

```text
tiamud-server
  ├── HTTP + WebSocket en 127.0.0.1:8080
  ├── Telnet en 127.0.0.1:4000
  ├── cliente web embebido en el ejecutable
  ├── SQLite en data/tiamud.db
  ├── mundos compilados en data/worlds/
  ├── assets publicados en data/assets/
  └── logs en data/logs/
```

Requisitos externos: ninguno.

### 5.2 Perfil B: servidor público en un único equipo

```text
Internet
  │
  ├── HTTPS/WSS ──► Caddy opcional ──► tiamud-server
  ├── Telnet TLS ─────────────────────► tiamud-server
  └── Telnet clásico opcional ────────► tiamud-server

data/
  ├── tiamud.db
  ├── assets/
  ├── worlds/
  ├── backups/
  └── logs/
```

Caddy o un proxy equivalente simplifica HTTPS y certificados, pero no forma parte del motor ni es necesario para jugar en local.

El proyecto proporcionará:

- Unidad de `systemd` de ejemplo.
- Configuración mínima de Caddy.
- Comando de backup.
- Comando de comprobación de salud.
- Guía para actualizar el binario y revertirlo.

### 5.3 Perfil C: crecimiento futuro

Solo se implementará si las métricas lo justifican:

- PostgreSQL en lugar de SQLite.
- Object storage o CDN en lugar de filesystem.
- Gateway de conexiones separado del proceso de mundo.
- Varias instancias o mundos en procesos distintos.

La posibilidad de este perfil influye en las interfaces internas, pero no añade dependencias al MVP.

## 6. Arquitectura de un solo proceso

```text
┌───────────────────────────────────────────────────────────────┐
│                       tiamud-server                           │
│                                                               │
│  ┌─────────────┐  ┌──────────────┐  ┌─────────────────────┐  │
│  │ HTTP + WSS  │  │ Telnet/TLS   │  │ Admin/health local  │  │
│  └──────┬──────┘  └──────┬───────┘  └──────────┬──────────┘  │
│         └────────────────┬┴──────────────────────┘             │
│                          ▼                                    │
│                  Action Router                                │
│                          │                                    │
│                          ▼                                    │
│             World Actor / Simulation Core                     │
│                          │ DomainEvents                        │
│                          ▼                                    │
│              Perception + View Projection                     │
│                   ┌──────┴─────────┐                          │
│                   ▼                ▼                          │
│              Web events       Text/GMCP                       │
│                                                               │
│  ┌────────────────┐  ┌─────────────────┐  ┌────────────────┐ │
│  │ SQLite         │  │ World registry  │  │ Static assets  │ │
│  │ persistence    │  │ scripts/packs   │  │ and web client │ │
│  └────────────────┘  └─────────────────┘  └────────────────┘ │
└───────────────────────────────────────────────────────────────┘
```

Un proceso no implica un único thread. El runtime puede usar varios threads para red, base de datos y tareas auxiliares, mientras el estado autoritativo de cada mundo conserva un único escritor lógico.

## 7. Capas del sistema

### 7.1 Definición del mundo

Contiene datos versionados e inmutables durante una publicación:

- Habitaciones y topología.
- Prototipos de objetos y NPC.
- Reglas, habilidades y tablas.
- Scripts.
- Definiciones de escenas.
- Referencias lógicas a assets.
- Ayudas y localización.

### 7.2 Estado de ejecución

Contiene:

- Personajes e inventarios.
- Instancias de objetos y NPC.
- Combate y efectos.
- Clima y tiempo.
- Estados de quests.
- Spawns y timers.
- Canales, grupos y sesiones.

### 7.3 Percepción

Determina qué conoce un personaje:

- Luz, ceguera y sentidos.
- Invisibilidad y ocultación.
- Puertas y salidas secretas.
- Estado de quests.
- Instancia del mundo.
- Clasificación de contenido y preferencias.
- Información administrativa.

Todo filtrado se realiza antes de serializar, registrar o renderizar una vista de jugador.

### 7.4 Presentación

Añade pistas no autoritativas:

- Fondo visual.
- Punto focal.
- Paleta o tema.
- Mood.
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
2: combate, mensajes y chat
3: estado de UI
4: visuales y audio
5: precarga
```

Al aplicar presión:

- Se coalescen actualizaciones de vitals y estado visual.
- Se descartan precargas.
- Se reemplazan visuales obsoletos por el último.
- Nunca se descarta silenciosamente el resultado de un comando aceptado.
- Si la cola crítica no puede avanzar, se desconecta al cliente lento de forma explícita.

## 9. Stack del servidor

### 9.1 Elección principal

- Rust estable.
- Tokio para red, timers y tareas.
- Axum para HTTP y WebSocket.
- rustls para TLS directo cuando se configure.
- serde para serialización.
- rusqlite con una conexión poseída por un worker dedicado.
- tracing para logs y spans.
- clap para CLI.

### 9.2 Motivos

- Un único binario portable.
- Bajo consumo en reposo.
- Seguridad de memoria en parsers de red.
- Buen control de límites y backpressure.
- Sin garbage collector compartiendo tiempo con el tick del mundo.
- Posibilidad de reutilizar tipos entre servidor, compiladores y herramientas.

### 9.3 Alternativas

Si el equipo no dispone de experiencia suficiente en Rust, Go es la alternativa preferida. Evennia/Python es una opción válida para validar rápidamente un único juego, pero no sería la base predeterminada de este motor nuevo.

## 10. Persistencia local

SQLite será la base de datos predeterminada y la única obligatoria en v1.

Configuración:

- Modo WAL.
- Foreign keys activadas.
- Busy timeout acotado.
- Migraciones incluidas en el binario.
- Transacciones para cambios jugables importantes.
- Checkpoints controlados.
- Integridad comprobable mediante CLI.

Datos persistidos inmediatamente:

- Cuenta y credenciales.
- Propiedad o pérdida de objetos.
- Economía.
- Avance de personaje.
- Resultado de quests.
- Acciones administrativas.

Datos que pueden agruparse o reconstruirse:

- Posición visual temporal.
- Último estado mostrado.
- Precargas.
- Métricas agregadas.

Comandos operativos:

```text
tiamud-admin migrate --database <db> --backup <safety-backup>
tiamud-admin check
tiamud-admin backup create --database <db> --output <backup>
tiamud-admin backup verify --backup <backup>
tiamud-admin backup restore --backup <backup> --database <new-db>
```

No se harán escrituras dobles ni replicación artesanal entre SQLite y PostgreSQL. Se mantendrá una interfaz de repositorios estrecha para añadir PostgreSQL como backend alternativo seleccionable cuando exista una necesidad medida. Cada instalación tendrá un único backend autoritativo y dispondrá de una migración explícita si cambia de SQLite a PostgreSQL. JSONB podrá utilizarse en ese adaptador para metadatos flexibles, pero el modelo jugable portable seguirá siendo relacional.

## 11. Cliente web

### 11.1 Stack

- TypeScript.
- Svelte.
- Vite.
- CSS propio con variables de tema.
- Service Worker pequeño.
- Playwright para pruebas.

El resultado compilado se incrustará en el ejecutable del servidor. No habrá runtime Node en producción.

### 11.2 Interfaz MVP

- Login y selección de personaje.
- Feed narrativo.
- Línea de comandos con historial y autocompletado.
- Acciones contextuales accesibles.
- Título, descripción e ilustración de escena.
- Salidas y navegación.
- Vitals.
- Inventario y equipo.
- Chat, tell, who e ignore.
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
- Paneles secundarios en pestañas o drawers.
- Comando fijo sin cubrir la salida.
- Acciones frecuentes utilizables con una mano.

### 11.4 Rendimiento del cliente

- DOM virtualizado o recortado para el historial largo.
- Historial persistido limitado y configurable.
- Assets responsivos.
- No descargar audio ni imágenes grandes cuando el usuario los desactive.
- Respetar `Save-Data` y preferencias equivalentes cuando estén disponibles.
- Objetivo inicial de JavaScript menor de 250 KiB comprimidos, excluyendo assets del juego.

### 11.5 Accesibilidad

- Objetivo WCAG 2.2 AA.
- Navegación completa por teclado.
- Feed con `role="log"` y anuncios selectivos.
- No anunciar automáticamente cada línea de spam de combate.
- Texto alternativo contextual.
- Alto contraste y `forced-colors`.
- Tamaño de fuente configurable.
- `prefers-reduced-motion` y control manual.
- Modo lector de pantalla que desactiva arte de celdas y efectos decorativos.

La imagen no contendrá información jugable exclusiva.

## 12. Protocolo web

El navegador utilizará un protocolo propio sobre WebSocket, no GMCP encapsulado.

Envelope:

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
- Cache corta de resultados para evitar duplicados tras reconectar.
- Retención breve de eventos para reanudación.
- Snapshot completo si falta parte de la secuencia.
- Heartbeat y timeout.
- Validación mediante esquema y tipos del servidor.
- Autorización de cada acción.

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

El namespace propio se limitará inicialmente a:

```text
Tiamud.Scene 1
Tiamud.Assets 1
```

MTTS será la fuente para ANSI, UTF-8, 256 colores, truecolor y lector de pantalla. Solo capacidades no cubiertas, como un panel multimedia específico, necesitarán negociación propia.

### 13.3 TinTin++

No se hará un fork en el MVP.

Se proporcionará:

- Script oficial para GMCP.
- Vitals y estado.
- Automapa.
- Alias y triggers de ejemplo.
- Manejo de URLs de escena sin descarga automática obligatoria.

### 13.4 Mudlet

Se priorizará un paquete oficial capaz de:

- Mostrar la imagen de escena.
- Mantener mapa y vitals.
- Cachear assets.
- Respetar preferencias multimedia.
- Degradar a GMCP estándar si el paquete no está instalado.

### 13.5 Arte y gráficos de terminal

- Texto y ANSI son la base.
- Arte de celdas será opt-in.
- Kitty, SIXEL e iTerm2 serán experimentales y opt-in.
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

Las actualizaciones parciales serán tipadas. No se utilizará una lista `changed` que duplique los datos ni JSON Patch genérico en v1.

## 15. Variantes y condiciones

Las condiciones de contenido usan un AST declarativo y validado:

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
- Assets ausentes.
- Posibles fugas de contenido secreto.

## 16. World packs e importación

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
- Flags desconocidos.
- Colisiones.
- Codificaciones dudosas.
- MOBPROG o scripts no soportados.
- Referencias a contenido cuya licencia no está declarada.

## 17. Scripting

La mayor parte del mundo debe poder expresarse mediante datos, condiciones y acciones predefinidas.

Para comportamiento especial se propone Lua embebido y versionado, con estas restricciones:

- Sin filesystem.
- Sin red.
- Sin ejecución de procesos.
- Sin carga de bibliotecas nativas.
- API basada en capacidades explícitas.
- Límite de memoria por ejecución o contexto.
- Límite de instrucciones/tiempo.
- Número máximo de eventos encadenados.
- Semillas controlables para pruebas.

Los scripts de builders se consideran contenido confiable del operador, pero aun así se limitan para evitar errores y saturación accidental.

La API incluirá inicialmente hooks como:

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

## 18. Assets y publicación multimedia

### 18.1 Almacenamiento v1

Los recursos publicados viven en el filesystem local:

```text
data/assets/sha256/ab/cd/<digest>.avif
data/assets/sha256/ab/cd/<digest>.webp
data/assets/sha256/ab/cd/<digest>.jpg
```

El servidor los sirve directamente con caché inmutable. S3 o CDN serán adaptadores futuros.

### 18.2 Pipeline

```text
fuente
  ↓
cuarentena y límites
  ↓
decodificación y re-encode aislado
  ↓
eliminación de metadatos
  ↓
variantes responsivas
  ↓
hash de cada salida
  ↓
manifiesto publicado
```

Se propone libvips para procesado offline.

Formatos:

- AVIF para navegador moderno.
- WebP como fallback moderno.
- JPEG como compatibilidad.
- PNG únicamente cuando se requiera transparencia o lossless.

Anchuras iniciales:

```text
640, 1280
```

Se añadirán 320, 960 o 1920 únicamente cuando los perfiles de dispositivos y activos reales demuestren que compensan el coste de almacenamiento y compilación.

Presupuestos orientativos:

- Imagen habitual: 150–500 KiB.
- Imagen excepcional: hasta 2 MiB.
- Dimensión máxima publicada: 4096 × 4096.
- Upload fuente: configurable y procesado fuera del actor de mundo.

### 18.3 Manifiesto

Cada asset lógico registrará:

- ID y revisión.
- Variantes y digests.
- Dimensiones.
- Punto focal.
- Color dominante o placeholder.
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
- Generar ideas para NPC o quests.
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
ai.provider = "compatible" # endpoint configurado por el operador
```

Para ejecución local se favorecerán modelos cuantizados pequeños, aproximadamente de 1B–4B parámetros, usados solo bajo demanda editorial.

El motor no descargará modelos automáticamente.

### 19.3 IA durante el juego

No forma parte del MVP. Si se experimenta posteriormente:

- Se ejecuta en un worker o proceso opcional.
- Nunca decide reglas, recompensas, daño, inventario ni permisos.
- Cola global y por jugador estrictamente acotada.
- Límite configurable de CPU, RAM, tokens y tiempo.
- Respuesta cacheable cuando sea apropiado.
- Fallback determinista si falla o está desactivada.
- Posibilidad de desactivar por mundo y por jugador.
- Etiquetado claro de contenido generado.

Un fallo del proveedor de IA no puede ralentizar movimiento, combate, chat ni guardado.

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
- Passkeys como mejora futura.
- Telnet TLS para credenciales en clientes compatibles.
- Telnet sin cifrar opt-in y con advertencia.

### 21.2 Límites

- Tamaño de mensaje WebSocket: 64 KiB inicialmente.
- Subnegociación Telnet acotada.
- Profundidad y tamaño JSON acotados.
- Frecuencia de comandos y login limitada.
- Máximo de sesiones por cuenta e IP configurable.
- Colas acotadas en todas las fronteras.
- Timeouts para DB, scripts, assets y shutdown.

### 21.3 Contenido y assets

- Sin SVG remoto en v1.
- Sin HTML o JavaScript arbitrario.
- Sin `data:` o `file:` URLs.
- Sin hosts externos no permitidos.
- MIME detectado por contenido y re-encode de imágenes.
- Metadatos eliminados.
- CSP y `X-Content-Type-Options: nosniff`.
- Procesado de uploads fuera del actor de mundo y con recursos limitados.

### 21.4 Moderación

Antes de una alpha pública:

- Ignore, mute y block.
- Reporte de usuarios y mensajes.
- Rate limiting de chat.
- Roles administrativos.
- Audit log.
- Bans temporales y permanentes.
- Protección de datos sensibles en logs.

## 22. Operación y observabilidad

Logs estructurados con rotación y salida legible para instalación simple.

Métricas mínimas:

- Conexiones por transporte.
- Sesiones autenticadas.
- Latencia de acciones.
- Longitud de colas.
- Visuales coalescidos o descartados.
- Tiempo de scripts y timeouts.
- Transacciones y errores SQLite.
- Uso de memoria.
- Tiempo de tick o ciclo de acciones.

El endpoint de salud distinguirá:

```text
/health/live
/health/ready
```

Shutdown elegante:

1. Dejar de aceptar conexiones.
2. Informar a los clientes.
3. Detener nuevas acciones.
4. Drenar acciones aceptadas dentro de un timeout.
5. Persistir estado necesario.
6. Cerrar SQLite.
7. Permitir que el cliente reconecte al reiniciar.

No se implementará `copyover` tradicional.

## 23. Repositorio propuesto

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

Podrá entregarse como un único comando `mud`; la separación del repositorio es modularidad de desarrollo, no microservicios.

## 24. Pruebas

### 24.1 Dominio

- Tests deterministas de combate, movimiento y percepción.
- Property tests de inventario, economía y efectos.
- Semillas reproducibles.
- Tests de invariantes después de cada acción.

### 24.2 Protocolos

- Fuzzing de Telnet e IAC.
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
- Variantes sin default.
- Texto alternativo ausente.
- Fugas de contenido oculto.
- Golden tests de renderizado de texto.

### 24.4 Web y accesibilidad

- Chromium, Firefox y WebKit mediante Playwright.
- Viewports móvil y escritorio.
- Navegación por teclado.
- Contraste y reduced motion.
- Lectores de pantalla mediante pruebas manuales periódicas.

### 24.5 Carga

- Bot headless que conecta, autentica, mueve, habla y combate.
- Soak test de 24 horas.
- Clientes que reconectan repetidamente.
- Assets ausentes o lentos.
- Scripts que consumen su cuota.

## 25. Roadmap

### Fase 0 — Decisiones y walking skeleton (2 semanas)

- Revisión legal y elección de licencia.
- Nombre provisional.
- Monorepo Rust/TypeScript.
- Un ejecutable con web embebida.
- SQLite y migración inicial.
- Una habitación compartida por navegador y Telnet.
- Bot de prueba.

Criterio:

> `tiamud-server` permite entrar desde navegador y Telnet al mismo mundo sin servicios externos.

### Fase 1 — Vertical browser-first (4–6 semanas)

- Cuenta registrada y selección de personaje.
- Personaje.
- Movimiento, look, say, tell, who y help.
- `scene.snapshot`.
- Imagen externa/local con fallback textual.
- Reconexión y snapshot.
- Cliente responsive.
- Texto Telnet limpio.

Criterio:

> Una pequeña aventura social se puede jugar desde navegador y terminal.

### Fase 2 — Bucle jugable (6–8 semanas)

- Inventario y equipo.
- Combate.
- Skills y efectos.
- NPC y spawns.
- Muerte y recuperación.
- Persistencia durable.
- Primeros hooks de script.
- Vitals e inventario estructurados.

Criterio:

> El jugador puede explorar, combatir, saquear, progresar y volver tras reiniciar el servidor.

### Fase 3 — Contenido y multimedia (4–6 semanas)

- World packs.
- `world validate/build/diff`.
- `asset build`.
- Variantes de imagen.
- Condiciones de escena.
- Importador histórico EnvyMUD/Merc parcial.
- Preview de texto, web y ANSI.

Criterio:

> Un área se valida, compila, publica y puede importarse parcialmente desde un formato histórico.

### Fase 4 — Alpha pública (6–8 semanas)

- Telnet TLS.
- GMCP estándar.
- Paquete Mudlet.
- Script TinTin++.
- Mapa y grupos.
- Moderación.
- Accesibilidad y móvil.
- Backup y restore probados.
- Métricas y pruebas de carga.

Criterio:

> El servidor puede mantenerse públicamente en un único equipo con procedimientos documentados.

### Fase 5 — Beta (4–6 semanas)

- Publicación y rollback de mundos.
- Localización preparada.
- Mejoras de onboarding.
- Hardening de seguridad.
- Documentación para builders y operadores.
- Decisión basada en métricas sobre PostgreSQL, object storage o separación de procesos.

## 26. Criterios globales de aceptación

1. El servidor local arranca con un único ejecutable.
2. No requiere conexión a Internet para iniciar y jugar en LAN/local.
3. Navegador y Telnet usan las mismas acciones de dominio.
4. El texto contiene toda la información jugable.
5. Una imagen ausente nunca bloquea una acción.
6. Ningún cliente recibe gráficos de terminal sin consentimiento.
7. No existen colas sin límite.
8. Una reconexión no duplica la última acción confirmada.
9. El cliente puede pedir un snapshot tras perder eventos.
10. Los IDs externos no revelan variantes ocultas.
11. SQLite puede respaldarse y restaurarse mediante la CLI.
12. El mundo puede actualizarse y revertirse de forma documentada.
13. Los scripts tienen cuotas de tiempo y recursos.
14. La IA puede estar completamente ausente.
15. Un fallo de IA no afecta al juego.
16. La instalación pública de un solo host está documentada.
17. El navegador funciona con teclado y lector de pantalla.
18. Telnet básico recibe texto limpio.
19. El proceso supera un soak test de 24 horas sin crecimiento continuo de memoria.
20. Cualquier paso hacia arquitectura distribuida exige métricas que lo justifiquen.

## 27. Decisiones abiertas en la propuesta original

- Lua frente a un lenguaje embebido puro Rust.
- ~~MPL-2.0 frente a Apache-2.0.~~ Resuelta por el ADR 0004 con la expresión
  dual `MIT OR Apache-2.0` para el software y `CC-BY-4.0` para contenido
  original.
- Método de emisión y renovación de certificados para Telnet TLS.
- ~~Primer mundo de ejemplo y licencia de su contenido.~~ Resuelta con Lantern
  Square y la licencia de contenido `CC-BY-4.0` registrada en el manifiesto y
  el ADR 0004.
- Objetivos de concurrencia realistas para el primer servidor público.
- Política de cuentas: password, magic link y/o passkeys.
- Nivel de compatibilidad requerido con clientes distintos de Mudlet y TinTin++.

## 28. Regla de evolución

Se empezará con la solución local más sencilla que mantenga límites y fronteras correctos.

Una dependencia o servicio nuevo solo se añade cuando:

1. Resuelve un problema observado.
2. Existe una métrica o caso reproducible.
3. No puede resolverse razonablemente dentro del proceso actual.
4. Incluye migración, operación y rollback documentados.

El objetivo no es construir infraestructura para un MMO hipotético, sino hacer un MUD moderno, rápido, mantenible y agradable de operar.

## 29. Decisiones cerradas el 15 de julio de 2026

Estas decisiones sustituyen cualquier alternativa contradictoria que permanezca en borradores anteriores:

1. Se construirá Tiamud como motor nuevo en Rust, inspirado por EnvyMUD, Merc y otros MUD de su época, pero sin copiar sus implementaciones históricas.
2. El producto será browser-first y text-complete. WebSocket será el transporte principal del navegador; Telnet textual seguirá siendo jugable.
3. El despliegue de referencia será un único proceso en un equipo corriente. Docker será opcional y Kubernetes, Redis, S3 y microservicios no formarán parte de v1.
4. El núcleo será un actor propietario del mundo. Las acciones de dominio serán independientes de HTTP, WebSocket y Telnet.
5. SQLite en WAL será el backend predeterminado mediante rusqlite y un worker dedicado. PostgreSQL podrá añadirse como backend alternativo, nunca como doble escritura obligatoria.
6. Las ilustraciones serán acompañamiento atmosférico. No contendrán información jugable exclusiva ni intentarán componer en tiempo real cada NPC u objeto.
7. El pipeline offline producirá AVIF como variante preferente para navegador, WebP como fallback moderno y JPEG como compatibilidad. Las primeras anchuras serán 640 y 1280 píxeles.
8. Los fallos o ausencias de imágenes no bloquearán el juego. El procesamiento y la IA no se ejecutarán dentro del actor de mundo.
9. La IA será una ayuda declarada para desarrollo y, opcionalmente, para autoría offline ligera. No será dependencia de ejecución y toda salida editorial requerirá revisión humana.
10. Las áreas históricas EnvyMUD/Merc legalmente utilizables servirán para ayudar a migrar contenido mediante el importador offline independiente `envy-import`. WorldPack seguirá siendo el formato canónico de Tiamud.
11. La reconexión pedirá un snapshot autoritativo; v1 no implementará replay general de comandos ni eventos.
12. Las decisiones permanentes y el estado de implementación se conservarán dentro del repositorio, junto con esta especificación histórica versionada.
