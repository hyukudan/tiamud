# Documentación de Tiamud

Este directorio reúne la documentación técnica y de proyecto de Tiamud:
contrato entre agentes, especificación, guías de autoría, decisiones de
arquitectura, diseño, operación, protocolo y meta-documentos de estado.

## Orden de orientación

1. [`../AGENTS.md`](../AGENTS.md) — contrato canónico entre agentes.
2. [`project/PROJECT_MEMORY.md`](project/PROJECT_MEMORY.md) — memoria e
   historia del proyecto.
3. [`project/COMPLETION_MATRIX.md`](project/COMPLETION_MATRIX.md) — matriz de
   finalización.
4. ADR pertinentes en [`decisions/`](decisions/).
5. El diseño/especificación pertinente y la implementación y pruebas
   actuales.

## Fuente única de verdad

`project/COMPLETION_MATRIX.md` es la AUTORIDAD ÚNICA para los conteos de
pruebas, las versiones (WorldPack/AreaPack/snapshot/esquema SQL) y el estado
de cada fase (`Complete`/`Partial`). Ante conflicto entre la matriz y
cualquier texto de README, CLAUDE.md, AGENTS.md o un ADR antiguo, prevalece
la matriz. Los ADR aceptados siguen gobernando las decisiones de
arquitectura.

## Mapa de docs/

| Carpeta/Archivo | Propósito |
| --- | --- |
| [`../AGENTS.md`](../AGENTS.md) | Contrato canónico entre agentes: invariantes, verificación y límites honestos. |
| [`spec-v0.3.md`](spec-v0.3.md) | Especificación vigente (personajes, inventario, medios). |
| [`archive/spec-v0.2.md`](archive/spec-v0.2.md) | Especificación histórica en castellano (archivada; contexto, no vigente). |
| [`world-authoring.md`](world-authoring.md) | Guía de autoría de mundos (WorldPack/AreaPack). |
| [`authoring/`](authoring/) | Guía de autoría dividida por temas (README, media, mundos modulares, progresión/combate, validación/publicación, flujo). |
| [`decisions/`](decisions/) | Architecture Decision Records (ADR 0001–0022). Ver [`decisions/README.md`](decisions/README.md). |
| [`design/`](design/) | Documentos de diseño y hojas de ruta (progresión v3, compositor de escenas, hoja social/staff/loot). |
| [`operations/`](operations/) | Runbooks de operación (releases, despliegue en un único equipo, pruebas de soak). |
| [`protocol/`](protocol/) | Especificación de protocolo (`tiamud-v1.md`). |
| [`project/`](project/) | Meta-documentos de proyecto: `PROJECT_MEMORY.md`, `COMPLETION_MATRIX.md`, `NAME_DOMAIN_PRECHECK.md` y `log/`. |
| [`project/log/`](project/log/) | Registros fechados por fase/hito. Ver [`project/log/README.md`](project/log/README.md). |
| [`images/`](images/) | Recursos gráficos del README (logo, capturas). |
| [`reviews/`](reviews/) | Revisiones externas/auditoría (serie "Fable"). (No versionado como fuente de estado.) |
