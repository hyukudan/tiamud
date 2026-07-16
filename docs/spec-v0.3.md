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

Ambos adaptadores usan el esquema v9. La migración aditiva v8→v9 incorpora
suscripciones durables al conjunto cerrado `general`, `newbie`, `staff`; el
traslado SQLite→PostgreSQL copia la relación. La aceptación PostgreSQL v8
retenida precede esta migración y no se relabela como aceptación v9.

La operación está en [single-host.md](operations/single-host.md). Microservicios
y orquestación no son objetivo sin carga medida que lo justifique.

## Datos de mundo y autoría

WorldPack v3 es el objetivo canónico. AreaPack v2 permite composición modular
estricta. WorldPack v2, AreaPack v1 y Envy/Merc se normalizan al importar o
componer; el dominio v3 no conserva vocabulario heredado.

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
recibir `staff` requiere rol Moderator/Admin. La membresía del grupo sobrevive
a la desconexión, pero accept/revoke/kick/transfer resuelven un nombre conectado
y no administran un objetivo offline.

La misma política, rate limit, mute, ignore/block y buzones por sesión sirve a
acciones web, comandos Telnet y `Comm.Channel.Text` GMCP. No existen correo o
historial durable, shout/yell espacial, telepatía authored, follow/assist/
rescue, selección general de aliados, XP/misión compartida ni reparto de botín.

La autoridad de cuenta es independiente de la ficha. Los roles cerrados son
Player, Moderator, Admin; se asignan y nunca se ganan por XP, nivel, clase o
talentos. Moderación/cambios son durables y auditados. Títulos Angel/Semidiós/
Dios, perfiles granulares y comandos generales wizard siguen pendientes.

Los IDs de objeto describen objetos únicos con ubicación autoritativa. Los NPC
poseen botín fijo validado y lo transfieren al morir. No hay split prototipo/
instancia reutilizable, pilas, capacidad/peso, contenedores, give/trade,
rareza/nivel, afijos ni tablas ponderadas. Las recompensas repetibles necesitan
identidad durable y derechos de botín. Cajas de botín pagadas o tiradas con
moneda prémium quedan fuera del baseline.

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

`tiamud.v1` WebSocket sigue experimental. Clientes reciben IDs opacos salados,
revisiones y secuencias por sesión. Texto y percepción preceden cada
representación.

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
| WorldPack v3, validador, normalizador y catálogo | Integrado en servidor |
| Linaje/clase/talento/promoción y persistencia | Implementado y probado |
| Creación v3 navegador/Telnet | Default/none/explícito implementado |
| Ficha, talentos y promociones web | Implementado y probado |
| Recuperación dividida, frame gate y fallback GMCP | Implementado y probado |
| Compositor navegador y accesibilidad | Implementado y probado |
| Perfil de sprite transparente | WebP alfa + PNG implementado |
| Sidecar de entidades seguro | Implementado; sin sprites de muestra |
| Pistas de combate | Ataque/golpe/derrota NPC; sin jugador/heal/lápida |
| Social fase 1 y esquema v9 | Grupos/chat/reply/canales y paridad Web/Telnet/GMCP implementados; aceptación PostgreSQL v9 completa aún no retenida |
| Protocolo atómico de delta denso | Pendiente |

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
