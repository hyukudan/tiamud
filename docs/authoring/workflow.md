# Flujo de autoría en un único host

Tiamud se ejecuta en un equipo corriente. La autoría no necesita Kubernetes,
una granja de build ni un servicio de contenido vivo. El flujo normal usa el
repositorio, el CLI Rust y un directorio de assets revisable; después publica
artefactos inmutables en el registro local del servidor.

```text
AreaPack v8 + assets revisados
              |
              v
      componer y validar
              |
              v
    probar, previsualizar y comparar
              |
              v
      publicación inmutable 6A
              |
              v
 carga aditiva 6B, corte 6C o retiro interno 6D
```

## Bucle recomendado

1. Trabaja en la rama canónica `main` y edita el AreaPack propietario mínimo.
2. Compila assets raster sin conexión; registra procedencia, licencia y revisión.
3. Compón una candidata en una ruta nueva.
4. Valida, ejecuta pruebas embebidas y previsualiza las escenas afectadas.
5. Compara con el artefacto revisado mediante `diff`.
6. Ejecuta las pruebas del repositorio y solicita revisión humana.
7. Publica catálogo y bundles inmutables con `area-publish`.
8. Para un servidor en modo catálogo, envía SIGHUP solo durante una ventana en
   que pueda quedar sin sesiones; conserva el catálogo previo para rollback.

El runtime no compone áreas, invoca codificadores de imagen, consulta IA ni
descarga medios de terceros. Consulta la
[guía de operación](../operations/single-host.md) para servicio, base de datos,
TLS, copias y rollback.

Los AreaPack son un layout editorial multipack, no servicios desplegables. 6B
resuelve bundles locales bajo demanda dentro de una generación. 6C abre y
verifica la candidata completa fuera del actor y solo la activa con cero
sesiones y cero leases; un `busy` conserva la generación anterior. El corte no
migra datos: reglas, objetos iniciales, NPC, botín, afijos, áreas, ownership e
IDs estables deben ser idénticos. 6D solo retira/rehidrata internamente la misma
revisión de una unidad vacía; no existen reemplazo de revisión ni LRU operativo.

Para un rollback explícito, selecciona un catálogo inmutable retenido y solicita
la reconciliación local:

```sh
cargo run --locked -p world-pack --bin tiamud-world -- area-select \
  --catalog <ruta-exacta-del-catalogo-retenido> \
  --registry build/area-registry
sudo systemctl kill --signal=HUP tiamud
```

El servidor mantiene el lock cooperativo común con `area-publish`/`area-select`
durante el corte hasta resolver el CAS; no ejecutes herramientas que ignoren ese
lock sobre el registro confiable.

## Contenido histórico

Los ficheros EnvyMUD/Merc son material de origen sin conexión, no datos runtime.
Usa el [importador conservador](../../tools/envy-import/README.md), inspecciona
cada diagnóstico y resuelve procedencia y licencia. Los registros no soportados
quedan inertes y los programas importados nunca se ejecutan.

## IA opcional

La IA solo asiste a la autoría. El proveedor predeterminado es `none`; Ollama,
llama.cpp locales y endpoints HTTPS compatibles explícitos son opcionales. El
resultado es un borrador no confiable, no puede publicarse y requiere una
decisión humana separada. Consulta la
[guía de IA editorial](../../crates/editorial-ai/README.md).

## Frontera runtime

El servidor carga el WorldPack embebido, `TIAMUD_WORLD_PACK` o la pareja
`TIAMUD_AREA_REGISTRY` + `TIAMUD_AREA_CATALOG`. En modo catálogo, SQL v15 liga
la generación durable y prevalece en restart sobre `area-active`; SIGHUP es la
única reconciliación en vivo. `WorldRuntimeSnapshot` usa v8 y una
activación confirma snapshot + puntero mediante CAS antes del swap en memoria.

Al actualizar una instalación 6B/v13 con recovery compartido o personajes,
conserva G0 exacto en el primer arranque v14, configura
`TIAMUD_ACK_LEGACY_AREA_GENERATION=<sha256-exacto-G0>` y verifica su liga antes
de retirar la variable y publicar/seleccionar G1. V13 no guardaba la identidad;
cambiar ambos a la vez saltaría la comparación 6C.
Recovery vacío puede contener personajes. Sin liga, recovery compartido o
cualquier personaje obliga al ACK G0; cuentas solas sin personajes/recovery de
mundo pueden ligar sin ACK. Una autoridad monolítica no se convierte
implícitamente. Un commit ambiguo relee SQL y queda fail-stop con readiness 503
si no determina G0/G1. El ACK falla cerrado en modo monolítico, con liga, sin
recovery/personaje o ante digest distinto.

El contrato de artefacto permanece WorldPack v9/AreaPack v8/Project v2. Los
snapshots raw v1–v5 admiten 1 MiB; v6–v8, 2 MiB. No hay otra autoridad de
botín, inventario o trade ni doble escritura. La puerta 6C pasa 774 pruebas Rust
—329 del servidor— y la reauditoría 0 P1/0 P2. PostgreSQL 18 desechable pasa
20/20 pruebas del adaptador y 7/7 enfocadas del servidor, incluida la CAS v14;
el humo liga G0 y valida `noop`, readiness, Web, Telnet y cierre limpio. Véase
el [registro de aceptación PostgreSQL v14/PWA](../project/log/2026-07-17-postgresql-v14-and-pwa-acceptance.md).
