# ADR 0026: fuente tipada única de `tiamud.v1` y TypeScript generado

- Estado: aceptado — implementación parcial. Implementado de forma incremental en este árbol; no cambia el formato de cable, el límite de
  16 KiB ni ninguna decisión aceptada. Actualización 2026-10-15: las formas
  internas de `tiamud-core` (escena, jugador, objeto, mapa, evento y odds), la
  anotación de presentación y `action.perform` también se generan (puntos 3–5).
  2026-10-16: aceptado por el propietario.
- Fecha: 2026-10-15
- Alcance: protocolo navegador `tiamud.v1` (servidor `web.rs` y cliente
  `apps/web/src/lib/protocol.ts`)
- Relacionado: [ADR 0001](0001-project-foundation.md),
  [especificación del protocolo](../protocol/tiamud-v1.md)

## Contexto

El servidor construía unos 73 mensajes `tiamud.v1` con `serde_json::json!` ad
hoc en `crates/mud-server/src/web.rs` y el cliente declaraba a mano los mismos
tipos en `apps/web/src/lib/protocol.ts` (unas 3.000 líneas). No había
generación de código: un cambio de nombre, de opcionalidad (`null` frente a
campo omitido) o de vocabulario podía divergir entre ambos lados sin que una
prueba lo detectara antes de Playwright.

## Decisión

1. Un crate nuevo, `crates/protocol` (paquete `tiamud-protocol`), es la fuente
   única de los tipos serde del sobre v1, del vocabulario de tipos de mensaje
   servidor→cliente y cliente→servidor, de las cargas cliente→servidor salvo
   `action.perform`, y de las cargas servidor→cliente cuya forma pertenece al
   transporte (`server.*`, `action.result`, `sync.*`, `scene.patch`,
   `entity.*`, `vitals.update`, `inventory.patch`, `map.snapshot`,
   `container.contents`, `trade.contents`, `chat.message`, `media.*`, correo y
   tablones). Las cargas que incrustan proyecciones de `tiamud-core`
   (`SceneView`, `PlayerStateView`, vistas de objeto/mapa, eventos narrativos)
   son genéricas sobre esa proyección; `tiamud-protocol` no depende de
   `tiamud-core`.
2. `web.rs` construye todos los mensajes WebSocket con esos tipos y los
   convierte a `serde_json::Value` antes de encolarlos. Como `serde_json::Map`
   ordena las claves, los bytes son idénticos a la construcción anterior.
   Pruebas golden en `tiamud-protocol` y en `tiamud-server`
   (`web_protocol_golden_tests.rs`) comparan valor y bytes con copias literales
   de la construcción `json!` previa usando proyecciones reales del dominio.
3. Los tipos TypeScript se generan con [ts-rs](https://github.com/Aleph-Alpha/ts-rs)
   (MIT) en archivos versionados de `apps/web/src/lib/generated/`, uno por
   crate propietario del tipo: `protocol.ts` (`tiamud-protocol`), `core.ts`
   (proyecciones de `tiamud-core`: escena, jugador, objeto, mapa, evento
   narrativo, odds de recompensa y vocabularios cerrados de staff/canal) y
   `server.ts` (`tiamud-server`: `ActionPerformData` y la anotación de
   presentación de `runtime_content`). En los tres crates ts-rs es solo
   `dev-dependency` y el derive se aplica con `cfg_attr(test, ...)`: no entra
   en el grafo de release ni en `THIRD_PARTY_NOTICES.md`. Por eso cada crate
   genera solo sus propios tipos; `server.ts` nombra los vocabularios de
   `tiamud-core`/`tiamud-protocol` con `ts(type = ...)` y los importa de
   `core.ts`/`protocol.ts`. Los enteros de 64 bits se emiten como `number`.
   Ningún atributo serde cambia, así que el JSON de cable es idéntico.
4. Las pruebas `typescript::generated_typescript_is_current` (protocolo y
   núcleo) y `web::typescript::generated_typescript_is_current` (servidor)
   forman parte de `cargo test --workspace --locked` y fallan si el archivo
   versionado está obsoleto. Regenerar es explícito:
   `TIAMUD_UPDATE_GENERATED_PROTOCOL=1 cargo test --locked -p tiamud-protocol -p tiamud-core -p tiamud-server typescript`.
5. El cliente conserva toda su validación estricta en tiempo de ejecución. Los
   validadores devuelven los tipos generados; solo se refinan donde el cliente
   normaliza (`ActionResultData` convierte `message: null` en ausente,
   `ServerWelcomeData` y `MediaStopData` toleran campos ausentes, y tablones
   fijan los literales 90/1000 que el validador exige) o estrecha un campo que
   el servidor proyecta como cadena (rareza, ranura de equipo, estado de
   misión, tipo y objetivo de habilidad, capa de presentación) y convierte
   `nextCost`/`recoveryInTicks: null` en ausentes. `SceneImage` y la fuente
   visual añaden campos solo de cliente (PNG/`srcset` resueltos del
   manifiesto y fuentes reservadas), y la escena normalizada tolera
   `viewId`/`revision` ausentes.

## Consecuencias

- Un cambio de forma en el servidor rompe primero la compilación Rust, después
  la prueba de obsolescencia y finalmente `svelte-check`, en lugar de aparecer
  solo en Playwright.
- Desde la actualización del 2026-10-15 ya no quedan formas de cable de
  `tiamud.v1` duplicadas a mano en el cliente: `ActionPerformFields` es
  `Omit<ActionPerformData, 'requestId' | 'action'>` generado, y las formas de
  escena/jugador/objeto/mapa/evento/odds y `ScenePresentation` son los tipos
  generados con los refinamientos del punto 5. Siguen a mano, por diseño, esos
  refinamientos (vocabularios que el servidor proyecta como `String`) y la
  semántica tri-estado de `NullableField`, cuyo tipo TypeScript
  (`T | null` opcional) se declara con `ts(type = ...)`.
- La API HTTP de cuentas sigue usando `json!` y no forma parte de este ADR.
- Añadir un mensaje v1 exige ahora: tipo en `crates/protocol`, prueba golden,
  regeneración del TypeScript, validador estricto en `protocol.ts`, revisión
  Telnet/GMCP y Playwright, además de la política aditiva del protocolo.
