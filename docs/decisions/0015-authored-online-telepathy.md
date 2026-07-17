# ADR 0015: telepatía directa en línea como habilidad de autoría

- Estado: aceptada e implementada
- Fecha: 2026-07-17
- Alcance: comunicación ampliada, fase 5A
- Continúa: [hoja de ruta social, staff y botín](../design/social-staff-and-loot-roadmap.md)

## Contexto

Tiamud ya ofrece `tell` y `reply` como conversación privada entre personajes
conectados. Renombrar `tell` como telepatía no añadiría una mecánica: no
comprobaría que el personaje conoce una habilidad, no consumiría recursos, no
aplicaría cooldown ni permitiría que el mundo bloquee la comunicación.

La telepatía necesita compartir los límites, la moderación y los transportes
del sistema social sin convertir el texto de una conversación en estado
durable ni filtrar si el receptor ignora al emisor, está incapacitado o se
encuentra en un lugar protegido.

## Decisión

WorldPack v8 y AreaPack v7 añaden a una habilidad una comunicación opcional:

```json
{
  "communication": {
    "kind": "telepathy",
    "scope": "direct_online",
    "blockedRoomIds": ["tiamud:silent-vault"]
  }
}
```

WorldProject permanece en v2. `WorldRuntimeSnapshot` permanece en v7 y SQLite
y PostgreSQL en esquema v11: recursos, cooldowns y cooldown global ya forman
parte de `CharacterGameplaySnapshot` y se confirman en la transacción de acción
existente. No se añade una tabla de mensajes.

Una habilidad telepática:

- es `spell` o `utility`, con objetivo `single_ally`;
- tiene al menos un coste de recurso positivo y `cooldownTicks > 0`;
- no causa daño, cura ni estados;
- no puede pertenecer a un conjunto de habilidades de NPC;
- puede bloquearse en hasta 64 salas existentes; y
- cuenta dentro del máximo global de 32 habilidades de comunicación.

WorldPack v7/AreaPack v6 y versiones anteriores normalizan sin inventar
telepatía. Sus DTO estrictos rechazan `communication` inyectado.

## Ejecución y privacidad

La sintaxis textual es:

```text
telepathy <ability-id> <personaje> <mensaje>
```

Web usa la acción estructurada cerrada `social.telepathy` con `abilityId`,
`target` y `message`. Telnet presenta texto completo y GMCP reutiliza
`Comm.Channel.Text` con el canal `telepathy`; no aparece un paquete paralelo.
Una habilidad con `communication` se rechaza en la ruta genérica
`ability`/`combat.ability`: solo la acción telepática puede coordinar política
social, privacidad y entrega posterior al commit. Las cinco claves Web son
exactas; una clave adicional también se rechaza cuando su valor es `null`.

```mermaid
flowchart LR
    Input[Normalizar y resolver objetivo conectado] --> Actor[Validar habilidad y sala del emisor]
    Actor --> Policy[Mute · bucket · ignore/block · receptividad privada]
    Policy --> Checkpoint[Checkpoint del mundo]
    Checkpoint --> Spend[Consumir recurso y cooldowns]
    Spend --> Hook[OnTalk acotado]
    Hook --> Commit[Commit durable único]
    Commit -->|correcto| Deliver[Entrega best-effort al buzón online]
    Commit -->|falla| Rollback[Restaurar mundo; entregar cero]
```

El emisor debe estar vivo, fuera de recuperación y de stun, conocer la
habilidad, disponer de recursos y tener listos sus cooldowns; su sala no puede
estar bloqueada. Esos fallos rechazan antes de consumir.

El objetivo debe ser otro personaje conectado y resolverse sin ambigüedad. Que
el receptor ignore o bloquee, esté derrotado, recuperándose, aturdido, en una
sala bloqueada o haya perdido su buzón es información privada: la acción se
acepta y consume igual, pero no se entrega. La confirmación del emisor es la
misma en todos esos casos.

La entrega ocurre únicamente después del commit. Un error de núcleo, hook o
persistencia restaura recurso, cooldown de habilidad y cooldown global, y no
publica el mensaje. El `requestId` Web conserva el dedupe acotado existente.

## Retención y superficie visible

El mensaje y el destinatario no entran en `DomainEvent`, trazas, auditoría de
staff ni snapshots. La telepatía no actualiza `last_tell_senders`; `reply`
continúa respondiendo exclusivamente a un `tell` entregado. La entrega es
online y best-effort, no correo diferido.

`AbilityView.purpose` distingue `gameplay|telepathy`. El cliente oficial ofrece
campos nativos de personaje y mensaje para una habilidad telepática y no
muestra el selector de NPC. El receptor solo recibe canal, nombre visible del
emisor y texto acotado; nunca ID de habilidad, sala, identidad durable ni
motivo de una entrega suprimida.

Los límites existentes siguen vigentes: ID de habilidad hasta 64 bytes,
objetivo hasta 64 bytes, mensaje UTF-8 visible de 1 a 512 bytes, bucket común de
chat de ráfaga cuatro y reposición de uno por segundo, buzones por sesión de
256 y frames Web/Telnet acotados.

## Consecuencias

La telepatía pasa a ser una capacidad real del personaje y del mundo, sin
duplicar persistencia social ni debilitar privacidad. A cambio, el actor debe
coordinar una mutación de gameplay con una entrega social posterior al commit.

Quedan fuera de 5A la telepatía de grupo, los destinatarios desconectados, el
correo, el historial general, los emotes, shout/yell y los tablones. La
continuación prevista separa emotes authored (5B), correo offline con cuotas y
retención explícita (5C) y tablones moderables (5D).
